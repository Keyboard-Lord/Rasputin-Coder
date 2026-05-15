#!/usr/bin/env python3
"""Stress-test Rasputin's explicit 15-document artifact contract handling."""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from collections import Counter
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
RUNS_ROOT = ROOT / "benchmark_runs" / "stress_15_docs_contract"
DEFAULT_ENDPOINT = "http://127.0.0.1:11434"

REQUIRED_FILES = [
    "docs/01_PROJECT_OVERVIEW.md",
    "docs/02_ARCHITECTURE.md",
    "docs/03_TECHNOLOGY_STACK.md",
    "docs/04_CORE_CONCEPTS.md",
    "docs/05_FOLDER_STRUCTURE.md",
    "docs/06_MAIN_WORKFLOWS.md",
    "docs/07_API_REFERENCE.md",
    "docs/08_DATA_MODEL.md",
    "docs/09_CONFIGURATION.md",
    "docs/10_DEVELOPMENT_GUIDE.md",
    "docs/11_TESTING_STRATEGY.md",
    "docs/12_DEPLOYMENT_AND_OPERATIONS.md",
    "docs/13_SECURITY_AND_COMPLIANCE.md",
    "docs/14_KNOWN_LIMITATIONS_AND_TRADEOFFS.md",
    "docs/15_FUTURE_ROADMAP_AND_EXTENSIBILITY.md",
]

FILE_PURPOSES = {
    "docs/01_PROJECT_OVERVIEW.md": "explain the product scope, operator split, and expected outcomes",
    "docs/02_ARCHITECTURE.md": "describe the runtime architecture, core components, and execution boundaries",
    "docs/03_TECHNOLOGY_STACK.md": "inventory the languages, crates, tools, and external dependencies in use",
    "docs/04_CORE_CONCEPTS.md": "define execution state, execution outcome, audit logging, checkpoints, and sealing concepts",
    "docs/05_FOLDER_STRUCTURE.md": "map the repository layout and explain where major responsibilities live",
    "docs/06_MAIN_WORKFLOWS.md": "walk through the primary execution, validation, recovery, and operator workflows",
    "docs/07_API_REFERENCE.md": "document the important public interfaces, entrypoints, commands, and runtime surfaces",
    "docs/08_DATA_MODEL.md": "describe the durable records, state objects, event payloads, and artifact contracts",
    "docs/09_CONFIGURATION.md": "explain environment variables, workspace config, model config, and runtime knobs",
    "docs/10_DEVELOPMENT_GUIDE.md": "show how to build, run, test, and safely change the system",
    "docs/11_TESTING_STRATEGY.md": "explain deterministic coverage, benchmark coverage, stress harnesses, and validation gates",
    "docs/12_DEPLOYMENT_AND_OPERATIONS.md": "describe local operation, monitoring, troubleshooting, and upgrade workflow",
    "docs/13_SECURITY_AND_COMPLIANCE.md": "cover fail-closed behavior, trust boundaries, auditability, and operator controls",
    "docs/14_KNOWN_LIMITATIONS_AND_TRADEOFFS.md": "capture current limitations, non-goals, and architectural tradeoffs",
    "docs/15_FUTURE_ROADMAP_AND_EXTENSIBILITY.md": "outline next hardening steps, extension points, and roadmap direction",
}

PROMPT_FIDELITY_FRAGMENTS = [
    "Produce exactly 15 canonical markdown documents",
    "docs/01_PROJECT_OVERVIEW.md",
    "docs/15_FUTURE_ROADMAP_AND_EXTENSIBILITY.md",
    "Do not invent APIs, tests, commands, configuration, or workflows",
    "Do not stop after creating a subset",
]

GENERIC_DRIFT_PHRASES = [
    "begin your analysis now",
    "analyze goal requirements",
    "create documentation",
]


@dataclass
class RunArtifacts:
    run_dir: Path
    workspace: Path
    stdout_path: Path
    stderr_path: Path
    report_path: Path


def write(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def run_command(
    cmd: list[str],
    cwd: Path,
    env: dict[str, str] | None = None,
    timeout: int = 900,
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


def create_run_artifacts(mode_label: str) -> RunArtifacts:
    timestamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    run_dir = RUNS_ROOT / f"{timestamp}-{mode_label}"
    workspace = run_dir / "workspace"
    stdout_path = run_dir / "stdout.jsonl"
    stderr_path = run_dir / "stderr.txt"
    report_path = run_dir / "report.json"
    workspace.mkdir(parents=True, exist_ok=True)
    return RunArtifacts(run_dir, workspace, stdout_path, stderr_path, report_path)


def create_workspace(workspace: Path, model: str | None) -> None:
    if workspace.exists():
        shutil.rmtree(workspace)
    workspace.mkdir(parents=True, exist_ok=True)

    write(
        workspace / "README.md",
        "# Mini Rasputin Fixture\n\n"
        "This repository is a compact terminal-native autonomous coding system fixture.\n\n"
        "It has a reducer-driven execution state, an authoritative execution outcome, "
        "append-only audit logging, bounded self-healing recovery, and a clean operator split.\n"
        "The goal of this fixture is to provide enough grounded detail for documentation generation tests.\n",
    )
    write(
        workspace / "Cargo.toml",
        "[package]\n"
        "name = \"mini_rasputin_fixture\"\n"
        "version = \"0.1.0\"\n"
        "edition = \"2021\"\n\n"
        "[lib]\n"
        "path = \"src/lib.rs\"\n",
    )
    write(
        workspace / "src" / "lib.rs",
        "pub mod audit;\n"
        "pub mod runtime;\n"
        "pub mod state;\n\n"
        "pub use audit::AuditRecord;\n"
        "pub use runtime::ExecutionRuntime;\n"
        "pub use state::{ExecutionOutcome, ExecutionState};\n",
    )
    write(
        workspace / "src" / "state.rs",
        "#[derive(Debug, Clone, PartialEq, Eq)]\n"
        "pub enum ExecutionState {\n"
        "    Idle,\n"
        "    Planning,\n"
        "    Executing,\n"
        "    Validating,\n"
        "    Done,\n"
        "    Failed,\n"
        "    Blocked,\n"
        "}\n\n"
        "#[derive(Debug, Clone, PartialEq, Eq)]\n"
        "pub struct ExecutionOutcome {\n"
        "    pub terminal_state: ExecutionState,\n"
        "    pub confidence: u8,\n"
        "    pub summary: String,\n"
        "}\n",
    )
    write(
        workspace / "src" / "audit.rs",
        "#[derive(Debug, Clone, PartialEq, Eq)]\n"
        "pub struct AuditRecord {\n"
        "    pub sequence: u64,\n"
        "    pub event_type: String,\n"
        "    pub message: String,\n"
        "}\n",
    )
    write(
        workspace / "src" / "runtime.rs",
        "use crate::audit::AuditRecord;\n"
        "use crate::state::{ExecutionOutcome, ExecutionState};\n\n"
        "#[derive(Debug, Clone)]\n"
        "pub struct ExecutionRuntime {\n"
        "    pub state: ExecutionState,\n"
        "    pub outcome: Option<ExecutionOutcome>,\n"
        "    pub audit_log: Vec<AuditRecord>,\n"
        "    pub checkpoint_hash: String,\n"
        "    pub completion_confidence: u8,\n"
        "}\n\n"
        "impl ExecutionRuntime {\n"
        "    pub fn new() -> Self {\n"
        "        Self {\n"
        "            state: ExecutionState::Idle,\n"
        "            outcome: None,\n"
        "            audit_log: Vec::new(),\n"
        "            checkpoint_hash: \"fixture-hash\".to_string(),\n"
        "            completion_confidence: 0,\n"
        "        }\n"
        "    }\n"
        "}\n",
    )
    write(
        workspace / "tests" / "smoke_test.rs",
        "use mini_rasputin_fixture::ExecutionRuntime;\n\n"
        "#[test]\n"
        "fn runtime_starts_idle() {\n"
        "    let runtime = ExecutionRuntime::new();\n"
        "    assert!(runtime.outcome.is_none());\n"
        "}\n",
    )
    write(
        workspace / "config" / "rasputin.toml",
        "mode = \"normal\"\n"
        "checkpoint_validation = true\n"
        "operator_inspector = true\n",
    )
    write(
        workspace / "docs_seed.md",
        "# Existing Notes\n\n"
        "The production system separates normal mode from operator-only inspection surfaces.\n"
        "Execution outcome remains the terminal source of truth.\n",
    )
    if model:
        write(
            workspace / "rasputin.json",
            json.dumps({"ollama_model": model}, indent=2) + "\n",
        )

    try:
        run_command(["git", "init", "-q"], workspace, timeout=30)
    except (FileNotFoundError, subprocess.TimeoutExpired):
        pass


def build_stress_prompt() -> str:
    lines = [
        "Produce exactly 15 canonical markdown documents for this repository.",
        "This is an explicit deliverable contract, not a generic analysis request.",
        "Create the files in docs/ with these precise filenames and purposes:",
    ]
    for index, path in enumerate(REQUIRED_FILES, start=1):
        lines.append(f"{index}. {path} - {FILE_PURPOSES[path]}.")
    lines.extend(
        [
            "Requirements:",
            "- All 15 files must exist.",
            "- All 15 files must be non-empty markdown.",
            "- Use the repository contents as grounding.",
            "- Do not invent APIs, tests, commands, configuration, or workflows that are not supported by the repo.",
            "- If the repo lacks information, say so explicitly inside the relevant document instead of hallucinating.",
            "- Each file should use a clear H1 title and specific repo-grounded sections.",
            "- Do not merge files, rename files, or skip files.",
            "- Do not stop after creating a subset.",
            "Completion contract:",
            "- Final success is true only when the exact 15-file set exists and is non-empty.",
            "- Partial output is not complete.",
        ]
    )
    return "\n".join(lines)


def parse_jsonl(output: str) -> tuple[list[dict[str, Any]], int]:
    events: list[dict[str, Any]] = []
    parse_errors = 0
    for line in output.splitlines():
        stripped = line.strip()
        if not stripped.startswith("{"):
            continue
        try:
            events.append(json.loads(stripped))
        except json.JSONDecodeError:
            parse_errors += 1
    return events, parse_errors


def relative_markdown_files(root: Path) -> list[str]:
    return sorted(
        str(path.relative_to(root)).replace("\\", "/")
        for path in root.rglob("*.md")
        if path.is_file()
    )


def inspect_workspace(workspace: Path) -> dict[str, Any]:
    docs_dir = workspace / "docs"
    existing_required = [path for path in REQUIRED_FILES if (workspace / path).exists()]
    missing_required = [path for path in REQUIRED_FILES if path not in existing_required]
    empty_required = [
        path
        for path in existing_required
        if not (workspace / path).read_text(encoding="utf-8").strip()
    ]
    markdown_in_docs = []
    if docs_dir.exists():
        markdown_in_docs = sorted(
            str(path.relative_to(workspace)).replace("\\", "/")
            for path in docs_dir.rglob("*.md")
            if path.is_file()
        )
    unexpected_markdown = [path for path in markdown_in_docs if path not in REQUIRED_FILES]
    return {
        "docs_exists": docs_dir.exists(),
        "existing_required_files": existing_required,
        "missing_required_files": missing_required,
        "empty_required_files": empty_required,
        "markdown_files_in_docs": markdown_in_docs,
        "unexpected_markdown_files": unexpected_markdown,
        "all_markdown_files": relative_markdown_files(workspace),
    }


def first_event(events: list[dict[str, Any]], event_type: str) -> dict[str, Any] | None:
    for event in events:
        if event.get("event_type") == event_type:
            return event
    return None


def collect_affected_paths(events: list[dict[str, Any]]) -> list[str]:
    touched: set[str] = set()
    for event in events:
        for path in event.get("affected_paths", []) or []:
            normalized = str(path).replace("\\", "/")
            if normalized:
                touched.add(normalized)
    return sorted(touched)


def detect_drift(
    events: list[dict[str, Any]],
    workspace_state: dict[str, Any],
    exit_code: int,
    prompt: str,
) -> dict[str, Any]:
    signals: list[str] = []
    start_event = first_event(events, "RUNTIME_START")
    start_message = ""
    if start_event:
        start_message = str(start_event.get("message", ""))
    start_lower = start_message.lower()
    prompt_lower = prompt.lower()

    missing_prompt_fragments = [
        fragment
        for fragment in PROMPT_FIDELITY_FRAGMENTS
        if fragment.lower() not in start_lower
    ]
    if missing_prompt_fragments:
        signals.append(
            "prompt_fidelity_loss: runtime start omitted prompt fragments "
            + ", ".join(missing_prompt_fragments)
        )

    if "15 canonical markdown documents" not in prompt_lower:
        signals.append("prompt_build_error: expected 15-doc contract text missing from script prompt")

    if "15" not in start_lower or "docs/01_project_overview.md" not in start_lower:
        signals.append("active_objective_missing_15_doc_contract")

    touched_paths = collect_affected_paths(events)
    touched_required = [
        required
        for required in REQUIRED_FILES
        if any(
            path == required or path.endswith(f"/{required.replace('/', os.sep)}") or path.endswith(f"/{required}")
            for path in touched_paths
        )
    ]
    if not touched_required and not workspace_state["existing_required_files"]:
        signals.append("no_required_artifact_activity_detected")

    all_messages = "\n".join(str(event.get("message", "")) for event in events).lower()
    if any(phrase in all_messages for phrase in GENERIC_DRIFT_PHRASES) and not touched_required:
        signals.append("runtime_drifted_into_generic_analysis")

    if exit_code == 0 and (
        workspace_state["missing_required_files"]
        or workspace_state["empty_required_files"]
        or workspace_state["unexpected_markdown_files"]
        or len(workspace_state["markdown_files_in_docs"]) != len(REQUIRED_FILES)
    ):
        signals.append("false_success_before_full_contract")

    if first_event(events, "COMPLETION_GATE_ACCEPT") and workspace_state["missing_required_files"]:
        signals.append("completion_gate_accepted_before_full_contract")

    return {
        "detected": bool(signals),
        "signals": signals,
        "runtime_start_message": start_message,
        "touched_required_files": touched_required,
        "touched_paths": touched_paths,
    }


def classify_failure(
    events: list[dict[str, Any]],
    workspace_state: dict[str, Any],
    drift: dict[str, Any],
    event_counts: Counter[str],
    exit_code: int,
) -> str | None:
    event_messages = "\n".join(str(event.get("message", "")) for event in events)

    if "Planner backend unavailable" in event_messages or "Planner timeout" in event_messages:
        return "planner backend timeout"

    if exit_code == 0 and (
        workspace_state["missing_required_files"]
        or workspace_state["empty_required_files"]
        or workspace_state["unexpected_markdown_files"]
        or len(workspace_state["markdown_files_in_docs"]) != len(REQUIRED_FILES)
    ):
        return "completion gate failure"

    if any(signal.startswith("prompt_fidelity_loss") for signal in drift["signals"]) or any(
        signal == "active_objective_missing_15_doc_contract" for signal in drift["signals"]
    ):
        return "prompt fidelity loss"

    if "runtime_drifted_into_generic_analysis" in drift["signals"]:
        return "runtime drift"

    created_count = len(workspace_state["existing_required_files"])
    if created_count == 0 and drift["touched_required_files"] == []:
        return "contract extraction failure"

    if event_counts.get("MAX_ITERATIONS_EXCEEDED", 0) > 0 and created_count > 0 and exit_code != 0:
        return "iteration cap limitation"

    if created_count > 0 and (
        workspace_state["missing_required_files"]
        or workspace_state["empty_required_files"]
        or workspace_state["unexpected_markdown_files"]
    ):
        return "CRUD failure"

    if exit_code != 0:
        return "runtime drift"

    return None


def score_run(
    workspace_state: dict[str, Any],
    drift: dict[str, Any],
    failure_class: str | None,
    exit_code: int,
) -> str:
    all_present = not workspace_state["missing_required_files"]
    all_non_empty = not workspace_state["empty_required_files"]
    exact_set = (
        len(workspace_state["markdown_files_in_docs"]) == len(REQUIRED_FILES)
        and not workspace_state["unexpected_markdown_files"]
    )

    if exit_code == 0 and all_present and all_non_empty and exact_set and not drift["detected"]:
        return "pass"

    if exit_code == 0 and (not all_present or not all_non_empty or not exact_set):
        return "fail"

    if failure_class in {"contract extraction failure", "runtime drift"}:
        return "fail"

    if len(workspace_state["existing_required_files"]) > 0 and exit_code != 0:
        return "partial"

    return "fail"


def mode_label(args: argparse.Namespace) -> str:
    if args.planner == "stub":
        return "stub"
    return f"model-{args.model.replace(':', '_')}"


def run_stress_test(args: argparse.Namespace) -> dict[str, Any]:
    worker = build_worker()
    prompt = build_stress_prompt()
    artifacts = create_run_artifacts(mode_label(args))
    create_workspace(artifacts.workspace, args.model)

    planner_type = args.planner
    if args.model:
        planner_type = "http"

    env = os.environ.copy()
    env["FORGE_OUTPUT_MODE"] = "jsonl"
    env["FORGE_PLANNER_ENDPOINT"] = args.endpoint
    env["FORGE_CSS_COMPRESSION"] = "true"
    env["FORGE_DATA_DIR"] = str(artifacts.run_dir / "forge_state")
    env["FORGE_PLANNER_TIMEOUT_SECONDS"] = str(args.planner_timeout_seconds)
    if args.model:
        env["FORGE_PLANNER_MODEL"] = args.model

    timeout = 180 if planner_type == "stub" else args.timeout
    result = run_command(
        [str(worker), prompt, str(args.max_iterations), planner_type],
        artifacts.workspace,
        env=env,
        timeout=timeout,
    )

    artifacts.stdout_path.write_text(result.stdout, encoding="utf-8")
    artifacts.stderr_path.write_text(result.stderr, encoding="utf-8")

    events, parse_errors = parse_jsonl(result.stdout)
    event_counts: Counter[str] = Counter(
        str(event.get("event_type", "UNKNOWN")) for event in events
    )
    workspace_state = inspect_workspace(artifacts.workspace)
    drift = detect_drift(events, workspace_state, result.returncode, prompt)
    failure_class = classify_failure(events, workspace_state, drift, event_counts, result.returncode)
    score = score_run(workspace_state, drift, failure_class, result.returncode)

    completion_messages = [
        str(event.get("message", ""))
        for event in events
        if str(event.get("event_type", "")).startswith("COMPLETION")
        or event.get("event_type") in {"MAX_ITERATIONS_EXCEEDED", "RUNTIME_COMPLETE"}
    ]
    report = {
        "status": score,
        "failure_classification": failure_class,
        "mode": "planner" if args.planner == "stub" else "model",
        "planner": planner_type,
        "model": args.model,
        "max_iterations": args.max_iterations,
        "endpoint": args.endpoint,
        "workspace": str(artifacts.workspace),
        "report_path": str(artifacts.report_path),
        "stdout_path": str(artifacts.stdout_path),
        "stderr_path": str(artifacts.stderr_path),
        "prompt": {
            "text": prompt,
            "required_count": len(REQUIRED_FILES),
            "required_files": REQUIRED_FILES,
        },
        "workspace_state": workspace_state,
        "drift": drift,
        "completion_outcome": {
            "process_exit_code": result.returncode,
            "jsonl_parse_errors": parse_errors,
            "event_counts": dict(event_counts),
            "runtime_complete_seen": event_counts.get("RUNTIME_COMPLETE", 0) > 0,
            "completion_gate_accept_seen": event_counts.get("COMPLETION_GATE_ACCEPT", 0) > 0,
            "completion_gate_reject_count": event_counts.get("COMPLETION_GATE_REJECT", 0),
            "max_iterations_exceeded": event_counts.get("MAX_ITERATIONS_EXCEEDED", 0) > 0,
            "completion_messages": completion_messages,
        },
        "evidence": {
            "runtime_events": events,
        },
    }
    artifacts.report_path.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    return report


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Stress-test explicit 15-document contract execution"
    )
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument(
        "--planner",
        choices=["stub"],
        help="Run the stress test with the deterministic stub planner",
    )
    mode.add_argument(
        "--model",
        help="Run the stress test with the HTTP planner using the given model",
    )
    parser.add_argument(
        "--max-iterations",
        type=int,
        default=24,
        help="Maximum Forge iterations for the run",
    )
    parser.add_argument(
        "--endpoint",
        default=DEFAULT_ENDPOINT,
        help="Planner endpoint for HTTP runs",
    )
    parser.add_argument(
        "--timeout",
        type=int,
        default=1200,
        help="Timeout in seconds for live-model runs",
    )
    parser.add_argument(
        "--planner-timeout-seconds",
        type=int,
        default=120,
        help="Per-request planner timeout in seconds",
    )
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    report = run_stress_test(args)
    print(json.dumps(report, indent=2))

    status = report["status"]
    if status == "pass":
        raise SystemExit(0)
    if status == "partial":
        raise SystemExit(2)
    raise SystemExit(1)


if __name__ == "__main__":
    main()
