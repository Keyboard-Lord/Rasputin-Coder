#!/usr/bin/env python3
"""Stress-test Rasputin's working memory, follow-up routing, and chat-to-work chaining.

This script creates disposable workspaces and runs multi-turn sequences to validate
that follow-up prompts like "continue", "fix that", "do the rest" resolve against
working memory correctly without requiring full prompt restatement.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time
import uuid
from dataclasses import dataclass, field, asdict
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
RUNS_ROOT = ROOT / "benchmark_runs" / "continuity"
DEFAULT_ENDPOINT = "http://127.0.0.1:11434"


@dataclass
class TurnResult:
    """Result of a single turn in a multi-turn sequence."""
    turn_number: int
    user_input: str
    route_chosen: str
    resolved_task: str | None
    files_before: list[str]
    files_after: list[str]
    validation_passed: bool | None
    drift_flags: list[str]
    notes: str = ""


@dataclass
class ScenarioResult:
    """Result of a complete test scenario."""
    scenario_name: str
    passed: bool
    turns: list[TurnResult] = field(default_factory=list)
    final_score: str = "UNKNOWN"  # PASS, PARTIAL, FAIL
    summary: str = ""


@dataclass
class ContinuityReport:
    """Overall test report."""
    run_id: str
    timestamp: str
    planner_type: str
    overall_score: str
    scenarios: list[ScenarioResult] = field(default_factory=list)
    drift_summary: list[str] = field(default_factory=list)


def write(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def run_command(
    cmd: list[str],
    cwd: Path,
    env: dict[str, str] | None = None,
    timeout: int = 300,
) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        cmd,
        cwd=str(cwd),
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        timeout=timeout,
    )


def build_worker() -> Path:
    result = run_command(["cargo", "build", "-p", "forge_bootstrap"], ROOT, timeout=300)
    if result.returncode != 0:
        sys.stderr.write(result.stdout)
        sys.stderr.write(result.stderr)
        raise SystemExit("failed to build forge_bootstrap")
    return ROOT / "target" / "debug" / "forge_bootstrap"


def create_run_artifacts(planner_type: str) -> Path:
    timestamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    run_id = f"{timestamp}-planner-{planner_type}"
    run_dir = RUNS_ROOT / run_id
    run_dir.mkdir(parents=True, exist_ok=True)
    return run_dir


def create_workspace(run_dir: Path, model: str | None = None) -> Path:
    workspace = run_dir / "workspace"
    if workspace.exists():
        shutil.rmtree(workspace)
    workspace.mkdir(parents=True, exist_ok=True)
    
    # Basic Rust project fixture
    write(
        workspace / "README.md",
        "# Test Workspace\n\nA minimal fixture for continuity testing.\n",
    )
    write(
        workspace / "Cargo.toml",
        '[package]\nname = "continuity_test"\nversion = "0.1.0"\nedition = "2021"\n',
    )
    write(
        workspace / "src" / "lib.rs",
        "// Placeholder library\npub fn placeholder() {}\n",
    )
    
    if model:
        write(workspace / "rasputin.json", json.dumps({"ollama_model": model}, indent=2) + "\n")
    
    # Init git for grounding
    try:
        run_command(["git", "init", "-q"], workspace, timeout=30)
        run_command(["git", "add", "."], workspace, timeout=10)
        run_command(["git", "commit", "-m", "Initial", "-q"], workspace, timeout=10)
    except Exception:
        pass
    
    return workspace


def list_files(workspace: Path) -> list[str]:
    """List all files in workspace (excluding .git)."""
    files = []
    for f in workspace.rglob("*"):
        if f.is_file() and ".git" not in str(f):
            files.append(str(f.relative_to(workspace)))
    return sorted(files)


def read_file(workspace: Path, path: str) -> str | None:
    """Read file content if it exists."""
    try:
        return (workspace / path).read_text(encoding="utf-8")
    except Exception:
        return None


def parse_jsonl_events(stdout: str) -> list[dict]:
    """Parse JSONL output from Forge worker."""
    events = []
    for line in stdout.strip().split("\n"):
        line = line.strip()
        if not line:
            continue
        try:
            events.append(json.loads(line))
        except json.JSONDecodeError:
            pass
    return events


def detect_route_from_events(events: list[dict], user_input: str) -> str:
    """Detect what route the input took based on runtime events."""
    # Check for working memory / continuity events
    for event in events:
        event_type = event.get("event_type", "")
        payload = event.get("payload", {})
        
        if event_type == "continuity":
            if "follow-up resolved" in str(payload):
                return "FOLLOW_UP"
        if event_type == "autonomy":
            if "plain text goal detected" in str(payload):
                return "TASK_GOAL"
        if event_type == "health":
            # Chat mode does health checks
            continue
    
    # Check for task execution patterns
    has_tool_calls = any(e.get("event_type") == "tool_call" for e in events)
    has_completion = any(e.get("event_type") == "completion_gate" for e in events)
    
    if has_tool_calls or has_completion:
        return "TASK_EXECUTION"
    
    # Default assumption
    return "UNKNOWN"


def extract_resolved_task_from_events(events: list[dict]) -> str | None:
    """Extract what task was actually executed after follow-up resolution."""
    for event in events:
        if event.get("event_type") == "continuity":
            payload = event.get("payload", "")
            # Parse "follow-up resolved: 'X' → task execution"
            match = re.search(r"follow-up resolved: '([^']+)'", str(payload))
            if match:
                return match.group(1)
        if event.get("event_type") == "task":
            return event.get("payload", {}).get("task")
    return None


def check_validation_result(events: list[dict]) -> bool | None:
    """Check if validation passed based on events."""
    for event in reversed(events):
        if event.get("event_type") == "validation_result":
            payload = event.get("payload", {})
            return payload.get("passed", False)
        if event.get("event_type") == "completion_gate":
            payload = event.get("payload", {})
            result = payload.get("result", "")
            if "Accept" in result:
                return True
            elif "Reject" in result:
                return False
    return None


def run_forge_turn(
    worker: Path,
    workspace: Path,
    task: str,
    env: dict[str, str],
    timeout: int = 120,
) -> tuple[list[dict], str, str, int, bool, str | None]:
    """Run a single turn with the Forge worker.
    
    Returns: (events, stdout, stderr, returncode, is_follow_up, resolved_task)
    
    Note: forge_bootstrap uses positional args: <task> [max_iterations] [planner_type]
    """
    # Get planner type from environment
    planner_type = env.get("FORGE_PLANNER_TYPE", "stub")
    
    cmd = [
        str(worker),
        task,           # arg 1: task
        "5",            # arg 2: max_iterations (capped for test speed)
        planner_type,   # arg 3: planner_type
    ]
    
    result = run_command(cmd, workspace, env=env, timeout=timeout)
    events = parse_jsonl_events(result.stdout)
    
    # Check stderr for continuity resolution
    is_follow_up = False
    resolved_task = None
    # Check for the resolved format: "Resolved 'continue' -> 'Continue working on: ...'"
    if "[RUNTIME] Continuity: Resolved" in result.stderr:
        is_follow_up = True
        import re
        match = re.search(r"Resolved '([^']+)' -> '(.+?)'(?:\n|$)", result.stderr, re.DOTALL)
        if match:
            resolved_task = match.group(2).strip()
            if len(resolved_task) > 100:
                resolved_task = resolved_task[:100] + "..."
    elif "[RUNTIME] Warning: Follow-up" in result.stderr:
        # It tried to be a follow-up but no previous state
        is_follow_up = True
    
    return events, result.stdout, result.stderr, result.returncode, is_follow_up, resolved_task


def run_scenario_artifact_contract_continuation(
    worker: Path,
    run_dir: Path,
    planner_type: str,
    model: str | None,
) -> ScenarioResult:
    """Scenario 1: Artifact contract continuation with 'continue'."""
    scenario_name = "artifact_contract_continuation"
    print(f"\n=== Running: {scenario_name} ===")
    
    workspace = create_workspace(run_dir, model)
    env = os.environ.copy()
    if planner_type == "stub":
        env["FORGE_PLANNER_TYPE"] = "stub"
    else:
        env["FORGE_PLANNER_TYPE"] = "ollama"
        env["FORGE_PLANNER_MODEL"] = model or "qwen2.5-coder:14b"
    
    turn_results = []
    drift_flags = []
    
    # Turn 1: Create a README file (simple task that works with stub planner)
    turn1_input = "Create README.md with project documentation"
    
    print(f"Turn 1: {turn1_input[:60]}...")
    files_before = list_files(workspace)
    events1, stdout1, stderr1, rc1, is_followup1, resolved1 = run_forge_turn(worker, workspace, turn1_input, env, timeout=120)
    files_after = list_files(workspace)
    
    route1 = "FOLLOW_UP" if is_followup1 else detect_route_from_events(events1, turn1_input)
    validation1 = check_validation_result(events1)
    
    turn1 = TurnResult(
        turn_number=1,
        user_input=turn1_input[:80],
        route_chosen=route1,
        resolved_task=resolved1 or turn1_input[:80],
        files_before=files_before,
        files_after=files_after,
        validation_passed=validation1,
        drift_flags=[],
        notes="Initial artifact contract request" if rc1 == 0 else f"Error: rc={rc1}",
    )
    turn_results.append(turn1)
    
    # Count files created
    files_created_turn1 = list_files(workspace)
    readme_exists_turn1 = "README.md" in files_created_turn1
    print(f"  Turn 1 created: {len(files_created_turn1)} files, README.md={readme_exists_turn1}")
    
    # Turn 2: Just say "continue"
    turn2_input = "continue"
    print(f"Turn 2: '{turn2_input}'")
    files_before = list_files(workspace)
    events2, stdout2, stderr2, rc2, is_followup2, resolved2 = run_forge_turn(worker, workspace, turn2_input, env, timeout=120)
    files_after = list_files(workspace)
    
    route2 = "FOLLOW_UP" if is_followup2 else detect_route_from_events(events2, turn2_input)
    validation2 = check_validation_result(events2)
    
    # Drift detection
    if route2 == "FOLLOW_UP":
        print(f"  ✓ Correctly routed as follow-up")
    else:
        drift_flags.append(f"Turn 2 routed as '{route2}' instead of 'FOLLOW_UP'")
        print(f"  ✗ Drift: routed as '{route2}'")
    
    if resolved2 and ("README" in resolved2 or "readme" in resolved2.lower()):
        print(f"  ✓ Resolved task references original contract")
    else:
        drift_flags.append(f"Turn 2 resolved task doesn't reference original contract: {resolved2}")
        print(f"  ✗ Drift: resolved task doesn't preserve context")
    
    files_created_turn2 = list_files(workspace)
    readme_exists_turn2 = "README.md" in files_created_turn2
    print(f"  Turn 2 total files: {len(files_created_turn2)}, README.md={readme_exists_turn2}")
    
    turn2 = TurnResult(
        turn_number=2,
        user_input=turn2_input,
        route_chosen=route2,
        resolved_task=resolved2[:100] if resolved2 else None,
        files_before=files_before,
        files_after=files_after,
        validation_passed=validation2,
        drift_flags=[d for d in drift_flags if "Turn 2" in d],
        notes="Follow-up 'continue'" if rc2 == 0 else f"Error: rc={rc2}",
    )
    turn_results.append(turn2)
    
    # Overall pass/fail - continuity works if follow-up was detected and task was resolved
    has_continuity = route2 == "FOLLOW_UP" and resolved2 is not None
    passed = has_continuity
    
    return ScenarioResult(
        scenario_name=scenario_name,
        passed=passed,
        turns=turn_results,
        final_score="PASS" if passed else "PARTIAL" if has_continuity else "FAIL",
        summary=f"README exists={readme_exists_turn2}, route={route2}, continuity={has_continuity}, drifts={len([d for d in drift_flags if 'Turn 2' in d])}",
    )


def run_scenario_finish_remaining(
    worker: Path,
    run_dir: Path,
    planner_type: str,
    model: str | None,
) -> ScenarioResult:
    """Scenario 2: Finish remaining files."""
    scenario_name = "finish_remaining"
    print(f"\n=== Running: {scenario_name} ===")
    
    workspace = create_workspace(run_dir, model)
    env = os.environ.copy()
    if planner_type == "stub":
        env["FORGE_PLANNER_TYPE"] = "stub"
    else:
        env["FORGE_PLANNER_TYPE"] = "ollama"
        env["FORGE_PLANNER_MODEL"] = model or "qwen2.5-coder:14b"
    
    turn_results = []
    drift_flags = []
    
    # Turn 1: Create partial set
    turn1_input = """Create exactly 4 config files:
1. config/app.toml
2. config/database.toml
3. config/cache.toml
4. config/logging.toml"""
    
    print(f"Turn 1: {turn1_input[:60]}...")
    files_before = list_files(workspace)
    events1, _, _, rc1, _, _ = run_forge_turn(worker, workspace, turn1_input, env, timeout=120)
    files_after = list_files(workspace)
    
    route1 = detect_route_from_events(events1, turn1_input)
    docs1 = [f for f in files_after if f.startswith("config/")]
    print(f"  Turn 1 created: {docs1}")
    
    turn_results.append(TurnResult(
        turn_number=1,
        user_input=turn1_input[:80],
        route_chosen=route1,
        resolved_task=turn1_input[:80],
        files_before=files_before,
        files_after=files_after,
        validation_passed=None,
        drift_flags=[],
        notes="Initial request",
    ))
    
    # Turn 2: "finish the remaining files"
    turn2_input = "finish the remaining files"
    print(f"Turn 2: '{turn2_input}'")
    files_before = list_files(workspace)
    events2, _, _, rc2, is_followup2, _ = run_forge_turn(worker, workspace, turn2_input, env, timeout=120)
    files_after = list_files(workspace)
    
    route2 = "FOLLOW_UP" if is_followup2 else detect_route_from_events(events2, turn2_input)
    resolved2 = extract_resolved_task_from_events(events2)
    
    docs2 = [f for f in files_after if f.startswith("config/")]
    print(f"  Turn 2 total: {docs2}")
    
    if route2 == "FOLLOW_UP":
        print(f"  ✓ Routed as follow-up")
    else:
        drift_flags.append(f"Routed as {route2} not FOLLOW_UP")
        print(f"  ✗ Drift: routed as {route2}")
    
    if len(docs2) >= 4:
        print(f"  ✓ All 4 config files present")
        passed = True
    else:
        drift_flags.append(f"Only {len(docs2)}/4 config files exist")
        print(f"  ✗ Only {len(docs2)}/4 files")
        passed = False
    
    turn_results.append(TurnResult(
        turn_number=2,
        user_input=turn2_input,
        route_chosen=route2,
        resolved_task=resolved2[:100] if resolved2 else None,
        files_before=files_before,
        files_after=files_after,
        validation_passed=None,
        drift_flags=drift_flags,
        notes="Finish remaining",
    ))
    
    return ScenarioResult(
        scenario_name=scenario_name,
        passed=passed,
        turns=turn_results,
        final_score="PASS" if passed else "FAIL",
        summary=f"Created {len(docs2)}/4 config files",
    )


def run_scenario_fix_that(
    worker: Path,
    run_dir: Path,
    planner_type: str,
    model: str | None,
) -> ScenarioResult:
    """Scenario 3: Fix that after a failure."""
    scenario_name = "fix_that"
    print(f"\n=== Running: {scenario_name} ===")
    
    workspace = create_workspace(run_dir, model)
    env = os.environ.copy()
    if planner_type == "stub":
        env["FORGE_PLANNER_TYPE"] = "stub"
    else:
        env["FORGE_PLANNER_TYPE"] = "ollama"
        env["FORGE_PLANNER_MODEL"] = model or "qwen2.5-coder:14b"
    
    turn_results = []
    drift_flags = []
    
    # Turn 1: Create code with intentional error
    turn1_input = "Create a function in src/calculator.rs that adds two numbers but has a compile error."
    
    print(f"Turn 1: {turn1_input[:60]}...")
    files_before = list_files(workspace)
    events1, _, _, rc1, _, _ = run_forge_turn(worker, workspace, turn1_input, env, timeout=120)
    files_after = list_files(workspace)
    
    # Check if calculator.rs was created
    calc_file = workspace / "src" / "calculator.rs"
    if calc_file.exists():
        content = calc_file.read_text()
        print(f"  Created calculator.rs ({len(content)} chars)")
    else:
        print(f"  Note: calculator.rs not created (may be in lib.rs)")
    
    turn_results.append(TurnResult(
        turn_number=1,
        user_input=turn1_input[:80],
        route_chosen=detect_route_from_events(events1, turn1_input),
        resolved_task=turn1_input[:80],
        files_before=files_before,
        files_after=files_after,
        validation_passed=None,
        drift_flags=[],
        notes="Create code with error",
    ))
    
    # Turn 2: "fix that"
    turn2_input = "fix that"
    print(f"Turn 2: '{turn2_input}'")
    files_before = list_files(workspace)
    events2, _, _, rc2, is_followup2, _ = run_forge_turn(worker, workspace, turn2_input, env, timeout=120)
    files_after = list_files(workspace)
    
    route2 = "FOLLOW_UP" if is_followup2 else detect_route_from_events(events2, turn2_input)
    resolved2 = extract_resolved_task_from_events(events2)
    
    print(f"  Route: {route2}")
    if resolved2:
        print(f"  Resolved: {resolved2[:80]}...")
    
    if route2 == "FOLLOW_UP":
        print(f"  ✓ Routed as follow-up")
    else:
        drift_flags.append(f"Routed as {route2} not FOLLOW_UP")
        print(f"  ✗ Drift: routed as {route2}")
    
    # For stub planner, we expect it to at least try
    # For live model, behavior may vary
    if planner_type == "stub":
        # Stub should handle follow-up
        passed = route2 == "FOLLOW_UP"
    else:
        # Live model - just check routing for now
        passed = route2 == "FOLLOW_UP"
    
    turn_results.append(TurnResult(
        turn_number=2,
        user_input=turn2_input,
        route_chosen=route2,
        resolved_task=resolved2[:100] if resolved2 else None,
        files_before=files_before,
        files_after=files_after,
        validation_passed=None,
        drift_flags=drift_flags,
        notes="Fix follow-up",
    ))
    
    return ScenarioResult(
        scenario_name=scenario_name,
        passed=passed,
        turns=turn_results,
        final_score="PASS" if passed else "FAIL",
        summary=f"Route={route2}, resolved={resolved2 is not None}",
    )


def run_scenario_improve(
    worker: Path,
    run_dir: Path,
    planner_type: str,
    model: str | None,
) -> ScenarioResult:
    """Scenario 4: Improve/refine existing work."""
    scenario_name = "improve_refine"
    print(f"\n=== Running: {scenario_name} ===")
    
    workspace = create_workspace(run_dir, model)
    env = os.environ.copy()
    if planner_type == "stub":
        env["FORGE_PLANNER_TYPE"] = "stub"
    else:
        env["FORGE_PLANNER_TYPE"] = "ollama"
        env["FORGE_PLANNER_MODEL"] = model or "qwen2.5-coder:14b"
    
    turn_results = []
    
    # Turn 1: Create a simple file
    turn1_input = "Create a simple README with basic project info."
    
    print(f"Turn 1: {turn1_input}")
    files_before = list_files(workspace)
    events1, _, _, rc1, _, _ = run_forge_turn(worker, workspace, turn1_input, env, timeout=120)
    files_after = list_files(workspace)
    
    readme_before = read_file(workspace, "README.md")
    print(f"  README created ({len(readme_before) if readme_before else 0} chars)")
    
    turn_results.append(TurnResult(
        turn_number=1,
        user_input=turn1_input,
        route_chosen=detect_route_from_events(events1, turn1_input),
        resolved_task=turn1_input,
        files_before=files_before,
        files_after=files_after,
        validation_passed=None,
        drift_flags=[],
        notes="Create initial README",
    ))
    
    # Turn 2: "make it cleaner"
    turn2_input = "make it cleaner"
    print(f"Turn 2: '{turn2_input}'")
    files_before = list_files(workspace)
    events2, _, _, rc2, is_followup2, _ = run_forge_turn(worker, workspace, turn2_input, env, timeout=120)
    files_after = list_files(workspace)
    
    route2 = "FOLLOW_UP" if is_followup2 else detect_route_from_events(events2, turn2_input)
    resolved2 = extract_resolved_task_from_events(events2)
    
    readme_after = read_file(workspace, "README.md")
    
    print(f"  Route: {route2}")
    if route2 == "FOLLOW_UP":
        print(f"  ✓ Routed as follow-up")
    else:
        print(f"  Note: routed as {route2}")
    
    # Check if README was modified
    changed = readme_before != readme_after
    if changed:
        print(f"  ✓ README was modified")
    else:
        print(f"  Note: README unchanged (may be acceptable)")
    
    turn_results.append(TurnResult(
        turn_number=2,
        user_input=turn2_input,
        route_chosen=route2,
        resolved_task=resolved2[:100] if resolved2 else None,
        files_before=files_before,
        files_after=files_after,
        validation_passed=None,
        drift_flags=[],
        notes=f"Improve request, README changed={changed}",
    ))
    
    passed = route2 == "FOLLOW_UP"
    
    return ScenarioResult(
        scenario_name=scenario_name,
        passed=passed,
        turns=turn_results,
        final_score="PASS" if passed else "PARTIAL",
        summary=f"Route={route2}, changed={changed}",
    )


def run_scenario_chat_escape(
    worker: Path,
    run_dir: Path,
    planner_type: str,
    model: str | None,
) -> ScenarioResult:
    """Scenario 5: Conversational question should not continue task."""
    scenario_name = "chat_escape_hatch"
    print(f"\n=== Running: {scenario_name} ===")
    
    workspace = create_workspace(run_dir, model)
    env = os.environ.copy()
    if planner_type == "stub":
        env["FORGE_PLANNER_TYPE"] = "stub"
    else:
        env["FORGE_PLANNER_TYPE"] = "ollama"
        env["FORGE_PLANNER_MODEL"] = model or "qwen2.5-coder:14b"
    
    turn_results = []
    drift_flags = []
    
    # Turn 1: Start a task
    turn1_input = "Create docs/intro.md with a brief introduction."
    
    print(f"Turn 1: {turn1_input}")
    files_before = list_files(workspace)
    events1, _, _, rc1, _, _ = run_forge_turn(worker, workspace, turn1_input, env, timeout=120)
    files_after = list_files(workspace)
    
    intro_exists = (workspace / "docs" / "intro.md").exists()
    print(f"  intro.md created: {intro_exists}")
    
    turn_results.append(TurnResult(
        turn_number=1,
        user_input=turn1_input,
        route_chosen=detect_route_from_events(events1, turn1_input),
        resolved_task=turn1_input,
        files_before=files_before,
        files_after=files_after,
        validation_passed=None,
        drift_flags=[],
        notes="Create intro.md",
    ))
    
    # Turn 2: Ask a conversational question
    turn2_input = "what is Rust ownership?"
    print(f"Turn 2: '{turn2_input}'")
    files_before = list_files(workspace)
    events2, _, _, rc2, is_followup2, _ = run_forge_turn(worker, workspace, turn2_input, env, timeout=120)
    files_after = list_files(workspace)
    
    route2 = "FOLLOW_UP" if is_followup2 else detect_route_from_events(events2, turn2_input)
    
    # Check no new files were created
    new_files = set(files_after) - set(files_before)
    
    print(f"  Route: {route2}")
    print(f"  New files: {new_files if new_files else 'none'}")
    
    # For this test, we want it to NOT be a follow-up
    # It should either be CHAT or UNKNOWN (or potentially error)
    is_chat = route2 in ("CHAT", "UNKNOWN")
    no_mutations = len(new_files) == 0
    
    if is_chat:
        print(f"  ✓ Correctly treated as chat/unknown")
    else:
        drift_flags.append(f"Question routed as {route2}, should be chat")
        print(f"  ✗ Drift: routed as {route2}")
    
    if no_mutations:
        print(f"  ✓ No workspace mutations")
    else:
        drift_flags.append(f"Workspace mutated on chat question: {new_files}")
        print(f"  ✗ Drift: workspace mutated")
    
    turn_results.append(TurnResult(
        turn_number=2,
        user_input=turn2_input,
        route_chosen=route2,
        resolved_task=None,
        files_before=files_before,
        files_after=files_after,
        validation_passed=None,
        drift_flags=drift_flags,
        notes="Chat question",
    ))
    
    passed = is_chat and no_mutations
    
    return ScenarioResult(
        scenario_name=scenario_name,
        passed=passed,
        turns=turn_results,
        final_score="PASS" if passed else "FAIL",
        summary=f"Route={route2}, mutations={len(new_files)}",
    )


def main() -> None:
    parser = argparse.ArgumentParser(description="Continuity stress test for Rasputin")
    parser.add_argument(
        "--planner",
        choices=["stub", "ollama"],
        default="stub",
        help="Planner type to test",
    )
    parser.add_argument(
        "--model",
        default="qwen2.5-coder:14b",
        help="Ollama model when using ollama planner",
    )
    parser.add_argument(
        "--scenario",
        choices=["all", "artifact", "finish", "fix", "improve", "chat"],
        default="all",
        help="Which scenario to run",
    )
    args = parser.parse_args()
    
    print("=" * 70)
    print("RASPUTIN CONTINUITY STRESS TEST")
    print("=" * 70)
    print(f"Planner: {args.planner}")
    if args.planner == "ollama":
        print(f"Model: {args.model}")
    print()
    
    # Build worker
    print("Building forge_bootstrap...")
    worker = build_worker()
    print(f"Worker: {worker}")
    
    # Create run artifacts
    run_dir = create_run_artifacts(args.planner)
    print(f"Run directory: {run_dir}")
    
    # Run scenarios
    results = []
    
    scenarios_to_run = {
        "artifact": args.scenario in ("all", "artifact"),
        "finish": args.scenario in ("all", "finish"),
        "fix": args.scenario in ("all", "fix"),
        "improve": args.scenario in ("all", "improve"),
        "chat": args.scenario in ("all", "chat"),
    }
    
    model = args.model if args.planner == "ollama" else None
    
    if scenarios_to_run["artifact"]:
        result = run_scenario_artifact_contract_continuation(worker, run_dir, args.planner, model)
        results.append(result)
    
    if scenarios_to_run["finish"]:
        result = run_scenario_finish_remaining(worker, run_dir, args.planner, model)
        results.append(result)
    
    if scenarios_to_run["fix"]:
        result = run_scenario_fix_that(worker, run_dir, args.planner, model)
        results.append(result)
    
    if scenarios_to_run["improve"]:
        result = run_scenario_improve(worker, run_dir, args.planner, model)
        results.append(result)
    
    if scenarios_to_run["chat"]:
        result = run_scenario_chat_escape(worker, run_dir, args.planner, model)
        results.append(result)
    
    # Compile report
    all_drift_flags = []
    for r in results:
        for turn in r.turns:
            all_drift_flags.extend(turn.drift_flags)
    
    passed_count = sum(1 for r in results if r.passed)
    total_count = len(results)
    
    if passed_count == total_count:
        overall_score = "PASS"
    elif passed_count >= total_count // 2:
        overall_score = "PARTIAL"
    else:
        overall_score = "FAIL"
    
    report = ContinuityReport(
        run_id=run_dir.name,
        timestamp=datetime.now(timezone.utc).isoformat(),
        planner_type=args.planner,
        overall_score=overall_score,
        scenarios=results,
        drift_summary=all_drift_flags,
    )
    
    # Write report
    report_path = run_dir / "report.json"
    report_path.write_text(json.dumps(asdict(report), indent=2, default=str))
    print(f"\nReport written: {report_path}")
    
    # Summary
    print("\n" + "=" * 70)
    print("SUMMARY")
    print("=" * 70)
    print(f"Overall Score: {overall_score}")
    print(f"Scenarios: {passed_count}/{total_count} passed")
    print()
    for r in results:
        status = "✓ PASS" if r.passed else "✗ FAIL"
        print(f"  {status}: {r.scenario_name}")
        print(f"         {r.summary}")
    
    if all_drift_flags:
        print("\nDrift Flags:")
        for flag in all_drift_flags:
            print(f"  ! {flag}")
    
    print()
    if overall_score == "PASS":
        print("All continuity tests passed!")
        sys.exit(0)
    elif overall_score == "PARTIAL":
        print("Some continuity tests passed with partial success.")
        sys.exit(0)
    else:
        print("Continuity tests failed - working memory needs hardening.")
        sys.exit(1)


if __name__ == "__main__":
    main()
