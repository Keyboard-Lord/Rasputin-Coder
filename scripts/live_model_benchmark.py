#!/usr/bin/env python3
"""Run Rasputin's gated live-model replacement benchmark.

This script is intentionally outside deterministic CI. It creates disposable
Rust workspaces, executes the real Forge worker with a live Ollama-backed
planner, captures JSONL event logs, validates artifacts independently, and
produces a replacement-readiness report.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from datetime import datetime
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
CORPUS = ROOT / "benchmarks" / "live_model" / "corpus.json"
DEFAULT_ENDPOINT = "http://127.0.0.1:11434"


@dataclass
class ValidatorResult:
    name: str
    passed: bool
    detail: str


def write(path: Path, content: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


def run_command(
    cmd: list[str],
    cwd: Path,
    env: dict[str, str] | None = None,
    timeout: int = 120,
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


def load_corpus(tier: str) -> list[dict[str, Any]]:
    data = json.loads(CORPUS.read_text(encoding="utf-8"))
    tasks = list(data["tasks"])
    if tier == "all":
        return tasks
    return [task for task in tasks if task.get("tier", "baseline") == tier]


def check_ollama(endpoint: str, model: str) -> tuple[bool, str, list[str]]:
    try:
        with urllib.request.urlopen(f"{endpoint}/api/tags", timeout=5) as response:
            payload = json.loads(response.read().decode("utf-8"))
    except (urllib.error.URLError, TimeoutError, json.JSONDecodeError) as exc:
        return False, f"Ollama unavailable at {endpoint}: {exc}", []

    models = sorted(item.get("name", "") for item in payload.get("models", []))
    if model not in models:
        return False, f"Model '{model}' is not installed. Installed: {', '.join(models)}", models
    return True, "ok", models


def build_worker() -> Path:
    result = run_command(["cargo", "build", "-p", "forge_bootstrap"], ROOT, timeout=180)
    if result.returncode != 0:
        sys.stderr.write(result.stdout)
        sys.stderr.write(result.stderr)
        raise SystemExit("failed to build forge_bootstrap")
    return ROOT / "target" / "debug" / "forge_bootstrap"


def create_workspace(task: dict[str, Any], workspace: Path, model: str) -> None:
    fixture = task["fixture"]
    workspace.mkdir(parents=True, exist_ok=True)
    write(workspace / "rasputin.json", json.dumps({"ollama_model": model}, indent=2) + "\n")

    if fixture == "rust_lib_basic":
        write_cargo(workspace, "bench_single_file")
        write(workspace / "src" / "lib.rs", "pub fn existing() -> &'static str {\n    \"ok\"\n}\n")
        write(workspace / "tests" / "double_tests.rs", "use bench_single_file::double;\n\n#[test]\nfn doubles_values() {\n    assert_eq!(double(4), 8);\n}\n")
    elif fixture == "rust_lib_feature":
        write_cargo(workspace, "bench_feature")
        write(workspace / "src" / "lib.rs", "pub fn existing() -> &'static str {\n    \"ok\"\n}\n")
        write(workspace / "tests" / "feature_tests.rs", "use bench_feature::math::{is_even, triple};\n\n#[test]\nfn feature_math_works() {\n    assert_eq!(triple(4), 12);\n    assert!(is_even(6));\n}\n")
    elif fixture == "rust_refactor":
        write_cargo(workspace, "bench_refactor")
        write(workspace / "src" / "lib.rs", "pub fn label_user(name: &str) -> String {\n    format!(\"user:{}\", name)\n}\n\npub fn label_team(name: &str) -> String {\n    format!(\"team:{}\", name)\n}\n")
        write(workspace / "tests" / "refactor_tests.rs", "use bench_refactor::{label_team, label_user};\n\n#[test]\nfn labels_are_stable() {\n    assert_eq!(label_user(\"ada\"), \"user:ada\");\n    assert_eq!(label_team(\"core\"), \"team:core\");\n}\n")
    elif fixture == "rust_bug":
        write_cargo(workspace, "bench_bug")
        write(workspace / "src" / "lib.rs", "pub fn add(a: i32, b: i32) -> i32 {\n    a - b\n}\n")
        write(workspace / "tests" / "bug_tests.rs", "use bench_bug::add;\n\n#[test]\nfn add_adds() {\n    assert_eq!(add(2, 3), 5);\n}\n")
    elif fixture == "rust_compile_error":
        write_cargo(workspace, "bench_compile")
        write(workspace / "src" / "lib.rs", "pub fn answer() -> i32 {\n    42\n\n#[cfg(test)]\nmod tests {\n    use super::*;\n\n    #[test]\n    fn answer_is_42() {\n        assert_eq!(answer(), 42);\n    }\n}\n")
    elif fixture == "rust_validation_failure":
        write_cargo(workspace, "bench_validation")
        write(workspace / "src" / "lib.rs", "pub fn normalize(input: &str) -> String {\n    input.to_string()\n}\n")
        write(workspace / "tests" / "validation_tests.rs", "use bench_validation::normalize;\n\n#[test]\nfn trims_and_lowercases() {\n    assert_eq!(normalize(\"  HeLLo  \"), \"hello\");\n}\n")
    elif fixture == "rust_find_change":
        write_cargo(workspace, "bench_find_change")
        write(workspace / "src" / "lib.rs", "pub mod config;\n")
        write(workspace / "src" / "config.rs", "pub const DEFAULT_TIMEOUT_MS: u64 = 1000;\n\npub fn default_timeout_ms() -> u64 {\n    DEFAULT_TIMEOUT_MS\n}\n")
        write(workspace / "tests" / "config_tests.rs", "use bench_find_change::config::default_timeout_ms;\n\n#[test]\nfn timeout_is_updated() {\n    assert_eq!(default_timeout_ms(), 5000);\n}\n")
    elif fixture == "rust_vertical_slice":
        write_cargo(workspace, "bench_vertical")
        write(workspace / "src" / "lib.rs", "pub fn existing() -> bool {\n    true\n}\n")
        write(workspace / "tests" / "settings_tests.rs", "use bench_vertical::parse_setting;\n\n#[test]\nfn parses_setting() {\n    let setting = parse_setting(\" name = Ada \").expect(\"setting\");\n    assert_eq!(setting.key, \"name\");\n    assert_eq!(setting.value, \"Ada\");\n}\n\n#[test]\nfn rejects_empty_key() {\n    assert!(parse_setting(\" = Ada \").is_none());\n}\n")
    elif fixture == "rust_expanded_nested_module":
        write_cargo(workspace, "bench_nested")
        write(workspace / "src" / "lib.rs", "pub mod auth;\npub mod config;\n")
        write(workspace / "src" / "auth" / "mod.rs", "pub mod password;\n")
        write(workspace / "src" / "auth" / "password.rs", "pub fn mask_password(input: &str) -> String {\n    \"*\".repeat(input.len().min(8))\n}\n")
        write(workspace / "src" / "config.rs", "pub fn service_name() -> &'static str {\n    \"bench-nested\"\n}\n")
        write(workspace / "tests" / "token_tests.rs", "use bench_nested::auth::token::{bearer_value, is_bearer};\n\n#[test]\nfn detects_bearer_header() {\n    assert!(is_bearer(\"Bearer abc123\"));\n    assert!(!is_bearer(\"Basic abc123\"));\n}\n\n#[test]\nfn extracts_bearer_value() {\n    assert_eq!(bearer_value(\"Bearer abc123\"), Some(\"abc123\".to_string()));\n    assert_eq!(bearer_value(\"Bearer   spaced\"), Some(\"spaced\".to_string()));\n    assert_eq!(bearer_value(\"Bearer\"), None);\n}\n")
    elif fixture == "rust_expanded_refactor":
        write_cargo(workspace, "bench_refactor_deep")
        write(workspace / "src" / "lib.rs", "pub mod audit;\npub mod team;\npub mod user;\n")
        write(workspace / "src" / "user.rs", "pub fn label_user(name: &str) -> String {\n    format!(\"user:{}\", name)\n}\n")
        write(workspace / "src" / "team.rs", "pub fn label_team(name: &str) -> String {\n    format!(\"team:{}\", name)\n}\n")
        write(workspace / "src" / "audit.rs", "pub fn label_audit(name: &str) -> String {\n    format!(\"audit:{}\", name)\n}\n")
        write(workspace / "tests" / "label_tests.rs", "use bench_refactor_deep::{audit::label_audit, team::label_team, user::label_user};\n\n#[test]\nfn labels_are_stable() {\n    assert_eq!(label_user(\"ada\"), \"user:ada\");\n    assert_eq!(label_team(\"core\"), \"team:core\");\n    assert_eq!(label_audit(\"login\"), \"audit:login\");\n}\n")
    elif fixture == "rust_expanded_bug_cart":
        write_cargo(workspace, "bench_cart")
        write(workspace / "src" / "lib.rs", "pub mod cart;\npub mod discount;\n")
        write(workspace / "src" / "cart.rs", "#[derive(Debug, Clone)]\npub struct CartItem {\n    pub sku: String,\n    pub price_cents: u32,\n    pub quantity: u32,\n}\n\npub fn subtotal_cents(items: &[CartItem]) -> u32 {\n    items.iter().map(|item| item.quantity).sum()\n}\n")
        write(workspace / "src" / "discount.rs", "pub fn apply_discount_cents(subtotal: u32, discount: u32) -> u32 {\n    subtotal.saturating_sub(discount)\n}\n")
        write(workspace / "tests" / "cart_tests.rs", "use bench_cart::cart::{subtotal_cents, CartItem};\n\n#[test]\nfn subtotal_multiplies_price_by_quantity() {\n    let items = vec![\n        CartItem { sku: \"a\".to_string(), price_cents: 250, quantity: 2 },\n        CartItem { sku: \"b\".to_string(), price_cents: 100, quantity: 3 },\n    ];\n    assert_eq!(subtotal_cents(&items), 800);\n}\n")
    elif fixture == "rust_expanded_find_change":
        write_cargo(workspace, "bench_settings")
        write(workspace / "src" / "lib.rs", "pub mod settings;\n")
        write(workspace / "src" / "settings" / "mod.rs", "pub mod defaults;\npub mod loader;\n")
        write(workspace / "src" / "settings" / "defaults.rs", "pub const DEFAULT_TIMEOUT_MS: u64 = 2500;\npub const DEFAULT_MAX_RETRIES: u8 = 3;\n")
        write(workspace / "src" / "settings" / "loader.rs", "use super::defaults::{DEFAULT_MAX_RETRIES, DEFAULT_TIMEOUT_MS};\n\n#[derive(Debug, Clone, PartialEq, Eq)]\npub struct RuntimeSettings {\n    pub timeout_ms: u64,\n    pub max_retries: u8,\n}\n\npub fn load_defaults() -> RuntimeSettings {\n    RuntimeSettings { timeout_ms: DEFAULT_TIMEOUT_MS, max_retries: DEFAULT_MAX_RETRIES }\n}\n")
        write(workspace / "tests" / "settings_tests.rs", "use bench_settings::settings::loader::load_defaults;\n\n#[test]\nfn retry_default_is_updated() {\n    let settings = load_defaults();\n    assert_eq!(settings.timeout_ms, 2500);\n    assert_eq!(settings.max_retries, 5);\n}\n")
    elif fixture == "rust_expanded_vertical_slice":
        write_cargo(workspace, "bench_settings_slice")
        write(workspace / "src" / "lib.rs", "pub mod validation;\n\n#[derive(Debug, Clone, PartialEq, Eq)]\npub struct Setting {\n    pub key: String,\n    pub value: String,\n}\n")
        write(workspace / "src" / "validation.rs", "pub fn is_valid_key(input: &str) -> bool {\n    !input.is_empty() && input.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_')\n}\n")
        write(workspace / "tests" / "settings_slice_tests.rs", "use bench_settings_slice::{parse_and_validate_setting, SettingError};\n\n#[test]\nfn parses_valid_setting() {\n    let setting = parse_and_validate_setting(\" service_name = Rasputin \").expect(\"setting\");\n    assert_eq!(setting.key, \"service_name\");\n    assert_eq!(setting.value, \"Rasputin\");\n}\n\n#[test]\nfn rejects_empty_key() {\n    assert_eq!(parse_and_validate_setting(\" = value \"), Err(SettingError::EmptyKey));\n}\n\n#[test]\nfn rejects_invalid_key() {\n    assert_eq!(parse_and_validate_setting(\"service-name=value\"), Err(SettingError::InvalidKey));\n}\n")
    else:
        raise ValueError(f"unknown fixture: {fixture}")


def write_cargo(workspace: Path, package_name: str) -> None:
    write(
        workspace / "Cargo.toml",
        f"[workspace]\n\n[package]\nname = \"{package_name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\npath = \"src/lib.rs\"\n",
    )


def run_validators(workspace: Path, validators: list[dict[str, Any]]) -> list[ValidatorResult]:
    results: list[ValidatorResult] = []
    for validator in validators:
        kind = validator["type"]
        if kind == "file_exists":
            path = workspace / validator["path"]
            results.append(ValidatorResult(f"file_exists:{validator['path']}", path.exists(), str(path)))
        elif kind == "file_contains":
            path = workspace / validator["path"]
            text = validator["text"]
            content = path.read_text(encoding="utf-8") if path.exists() else ""
            results.append(
                ValidatorResult(
                    f"file_contains:{validator['path']}",
                    text in content,
                    f"expected substring: {text}",
                )
            )
        elif kind == "file_not_contains":
            path = workspace / validator["path"]
            text = validator["text"]
            content = path.read_text(encoding="utf-8") if path.exists() else ""
            results.append(
                ValidatorResult(
                    f"file_not_contains:{validator['path']}",
                    text not in content,
                    f"forbidden substring: {text}",
                )
            )
        elif kind == "rust_prefix_helper_refactor":
            path = workspace / validator["path"]
            content = path.read_text(encoding="utf-8") if path.exists() else ""
            passed, detail = validate_rust_prefix_helper_refactor(content)
            results.append(ValidatorResult(f"rust_prefix_helper_refactor:{validator['path']}", passed, detail))
        elif kind == "cargo_test":
            result = run_command(["cargo", "test", "--quiet"], workspace, timeout=180)
            detail = (result.stdout + result.stderr).strip()[-2000:]
            results.append(ValidatorResult("cargo_test", result.returncode == 0, detail))
        elif kind == "cargo_check":
            result = run_command(["cargo", "check", "--quiet"], workspace, timeout=180)
            detail = (result.stdout + result.stderr).strip()[-2000:]
            results.append(ValidatorResult("cargo_check", result.returncode == 0, detail))
        else:
            results.append(ValidatorResult(kind, False, f"unknown validator type: {kind}"))
    return results


def validate_rust_prefix_helper_refactor(content: str) -> tuple[bool, str]:
    required_exports = ("pub fn label_user", "pub fn label_team")
    missing_exports = [item for item in required_exports if item not in content]
    if missing_exports:
        return False, f"missing expected public functions: {', '.join(missing_exports)}"

    if 'format!("user:{}"' in content or 'format!("team:{}"' in content:
        return False, "repeated direct prefix format calls remain"

    has_private_helper = (
        "fn " in content
        and "prefix: &str" in content
        and "name: &str" in content
        and "pub fn format" not in content
    )
    if not has_private_helper:
        return False, "expected a private helper taking prefix and name"

    helper_call_count = content.count('("user", name)') + content.count('("team", name)')
    if helper_call_count < 2:
        return False, "expected both label functions to call the helper with explicit prefixes"

    if 'format!("{}:{}"' not in content and "format!(\"{prefix}:{name}\")" not in content:
        return False, "expected helper to centralize prefix formatting"

    return True, "private prefix helper centralizes both label functions"


def parse_jsonl(output: str) -> tuple[list[dict[str, Any]], int]:
    events: list[dict[str, Any]] = []
    parse_errors = 0
    for line in output.splitlines():
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            events.append(json.loads(line))
        except json.JSONDecodeError:
            parse_errors += 1
    return events, parse_errors


def score_task(
    process: subprocess.CompletedProcess[str] | None,
    validators: list[ValidatorResult],
    events: list[dict[str, Any]],
    parse_errors: int,
    timed_out: bool,
) -> tuple[str, dict[str, Any]]:
    validators_passed = sum(1 for item in validators if item.passed)
    all_validators_passed = validators_passed == len(validators)
    runtime_success = process is not None and process.returncode == 0 and not timed_out
    event_types = [event.get("event_type", "") for event in events]
    audit_complete = parse_errors == 0 and "RUNTIME_INIT" in event_types and "RUNTIME_COMPLETE" in event_types
    validation_seen = any(event_type.startswith("VALIDATION_") for event_type in event_types)
    runtime_error = any(event_type == "RUNTIME_ERROR" for event_type in event_types)
    mutating_tool_succeeded = any(
        event.get("event_type") == "TOOL_SUCCESS"
        and event.get("tool") in {"write_file", "apply_patch", "delete_file"}
        for event in events
    )

    if runtime_success and all_validators_passed and audit_complete and not runtime_error:
        classification = "PASS"
    elif validators_passed > 0 or mutating_tool_succeeded:
        classification = "PARTIAL"
    else:
        classification = "FAIL"

    recovery_events = [
        event for event in events
        if any(token in event.get("event_type", "") for token in ("REPAIR", "RETRY", "RECOVERY"))
    ]
    failed_validation_events = [
        event for event in events
        if event.get("event_type") in {"VALIDATION_REJECT", "RUNTIME_ERROR", "TOOL_ERROR"}
        or event.get("severity") == "ERROR"
    ]

    trust = {
        "audit_log_complete": audit_complete,
        "jsonl_parse_errors": parse_errors,
        "validation_observed": validation_seen,
        "replay_consistency": "not_checked_worker_cli",
        "checkpoint_continuity": "not_checked_worker_cli",
        "no_runtime_error": not runtime_error,
    }
    recovery = {
        "failure_events": len(failed_validation_events),
        "recovery_events": len(recovery_events),
        "retry_stayed_within_policy": True,
    }
    detail = {
        "runtime_success": runtime_success,
        "validator_count": len(validators),
        "validators_passed": validators_passed,
        "event_count": len(events),
        "iterations_observed": max([event.get("iteration", 0) for event in events], default=0),
        "timed_out": timed_out,
        "mutating_tool_succeeded": mutating_tool_succeeded,
        "trust": trust,
        "recovery": recovery,
    }
    return classification, detail


def run_task(
    task: dict[str, Any],
    run_dir: Path,
    worker: Path,
    model: str,
    endpoint: str,
    planner: str,
    timeout: int,
) -> dict[str, Any]:
    workspace = run_dir / "workspaces" / task["id"]
    task_dir = run_dir / "tasks" / task["id"]
    task_dir.mkdir(parents=True, exist_ok=True)
    create_workspace(task, workspace, model)

    env = os.environ.copy()
    env.update(
        {
            "FORGE_OUTPUT_MODE": "jsonl",
            "FORGE_PLANNER_MODEL": model,
            "FORGE_PLANNER_ENDPOINT": endpoint,
            "FORGE_PLANNER_TEMPERATURE": "0.0",
            "FORGE_PLANNER_SEED": "42",
        }
    )

    command = [str(worker), task["task"], str(task["max_iterations"]), planner]
    started = time.monotonic()
    timed_out = False
    process: subprocess.CompletedProcess[str] | None
    try:
        process = run_command(command, workspace, env=env, timeout=timeout)
    except subprocess.TimeoutExpired as exc:
        timed_out = True
        process = None
        stdout = exc.stdout if isinstance(exc.stdout, str) else ""
        stderr = exc.stderr if isinstance(exc.stderr, str) else ""
    else:
        stdout = process.stdout
        stderr = process.stderr
    wall_clock = time.monotonic() - started

    write(task_dir / "stdout.jsonl", stdout)
    write(task_dir / "stderr.txt", stderr)
    write(task_dir / "task.json", json.dumps(task, indent=2) + "\n")

    events, parse_errors = parse_jsonl(stdout)
    validator_results = run_validators(workspace, task["validators"])
    classification, detail = score_task(process, validator_results, events, parse_errors, timed_out)

    result = {
        "id": task["id"],
        "category": task["category"],
        "task": task["task"],
        "workspace": str(workspace),
        "model": model,
        "planner": planner,
        "exit_code": None if process is None else process.returncode,
        "wall_clock_seconds": round(wall_clock, 2),
        "classification": classification,
        "validators": [item.__dict__ for item in validator_results],
        **detail,
    }
    write(task_dir / "result.json", json.dumps(result, indent=2) + "\n")
    return result


def summarize(
    results: list[dict[str, Any]],
    run_dir: Path,
    model: str,
    endpoint: str,
    models: list[str],
    tier: str,
) -> dict[str, Any]:
    total = len(results)
    passes = sum(1 for result in results if result["classification"] == "PASS")
    partials = sum(1 for result in results if result["classification"] == "PARTIAL")
    fails = sum(1 for result in results if result["classification"] == "FAIL")
    avg_retries = sum(result["recovery"]["recovery_events"] for result in results) / total if total else 0.0
    avg_interventions = 0.0
    pass_rate = passes / total if total else 0.0

    if pass_rate >= 0.85 and fails == 0:
        verdict = "practical daily replacement"
    elif pass_rate >= 0.65:
        verdict = "replacement for limited task classes"
    elif pass_rate + (partials / total if total else 0.0) >= 0.5:
        verdict = "usable with caveats"
    else:
        verdict = "not ready"

    summary = {
        "run_dir": str(run_dir),
        "model": model,
        "tier": tier,
        "endpoint": endpoint,
        "installed_models": models,
        "total": total,
        "pass": passes,
        "partial": partials,
        "fail": fails,
        "pass_rate": round(pass_rate, 3),
        "partial_rate": round(partials / total if total else 0.0, 3),
        "fail_rate": round(fails / total if total else 0.0, 3),
        "average_recovery_events": round(avg_retries, 2),
        "average_operator_interventions": avg_interventions,
        "final_replacement_verdict": verdict,
        "tasks": results,
    }
    write(run_dir / "summary.json", json.dumps(summary, indent=2) + "\n")
    write(run_dir / "report.md", render_report(summary))
    return summary


def render_report(summary: dict[str, Any]) -> str:
    lines = [
        "# Rasputin Live-Model Replacement Benchmark Report",
        "",
        f"- Model: `{summary['model']}`",
        f"- Tier: `{summary['tier']}`",
        f"- Tasks: {summary['total']}",
        f"- Pass: {summary['pass']} ({summary['pass_rate']:.0%})",
        f"- Partial: {summary['partial']} ({summary['partial_rate']:.0%})",
        f"- Fail: {summary['fail']} ({summary['fail_rate']:.0%})",
        f"- Average recovery events: {summary['average_recovery_events']}",
        f"- Average operator interventions: {summary['average_operator_interventions']}",
        f"- Verdict: **{summary['final_replacement_verdict']}**",
        "",
        "## Task Results",
        "",
        "| Task | Category | Result | Time | Validators | Trust |",
        "|---|---|---:|---:|---:|---|",
    ]
    for task in summary["tasks"]:
        trust = task["trust"]
        trust_text = (
            f"audit={trust['audit_log_complete']}, validation={trust['validation_observed']}, "
            f"runtime_error={not trust['no_runtime_error']}"
        )
        lines.append(
            f"| `{task['id']}` | {task['category']} | {task['classification']} | "
            f"{task['wall_clock_seconds']}s | {task['validators_passed']}/{task['validator_count']} | {trust_text} |"
        )
    lines.extend(
        [
            "",
            "## Notes",
            "",
            "- This benchmark runs the real Forge worker with live local-model planning.",
            "- JSONL audit logs are preserved per task.",
            "- Replay and TUI checkpoint continuity are marked `not_checked_worker_cli` in this worker-level harness and remain covered by deterministic TUI/runtime tests.",
        ]
    )
    return "\n".join(lines) + "\n"


def main() -> int:
    parser = argparse.ArgumentParser(description="Run Rasputin's gated live-model replacement benchmark.")
    parser.add_argument("--model", default="qwen2.5-coder:14b")
    parser.add_argument("--endpoint", default=DEFAULT_ENDPOINT)
    parser.add_argument("--planner", default="http", choices=["http", "stub"])
    parser.add_argument("--tier", default="baseline", choices=["baseline", "expanded", "all"])
    parser.add_argument("--timeout", type=int, default=420)
    parser.add_argument("--run-id", default=datetime.now().strftime("%Y%m%d-%H%M%S"))
    parser.add_argument("--task", action="append", help="Run only a specific task id. Can be repeated.")
    parser.add_argument("--max-tasks", type=int, default=None)
    args = parser.parse_args()

    ok, reason, models = check_ollama(args.endpoint, args.model)
    if args.planner == "http" and not ok:
        raise SystemExit(reason)

    worker = build_worker()
    run_dir = ROOT / "benchmark_runs" / "live_model" / args.run_id
    if run_dir.exists():
        shutil.rmtree(run_dir)
    run_dir.mkdir(parents=True)

    tasks = load_corpus(args.tier)
    if args.task:
        selected = set(args.task)
        tasks = [task for task in tasks if task["id"] in selected]
    if args.max_tasks is not None:
        tasks = tasks[: args.max_tasks]
    if not tasks:
        raise SystemExit("no benchmark tasks selected")

    results = []
    for index, task in enumerate(tasks, start=1):
        print(f"[{index}/{len(tasks)}] {task['id']} ({task['category']})")
        results.append(run_task(task, run_dir, worker, args.model, args.endpoint, args.planner, args.timeout))
        print(f"  -> {results[-1]['classification']} in {results[-1]['wall_clock_seconds']}s")

    summary = summarize(results, run_dir, args.model, args.endpoint, models, args.tier)
    print()
    print(f"Report: {run_dir / 'report.md'}")
    print(f"Verdict: {summary['final_replacement_verdict']}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
