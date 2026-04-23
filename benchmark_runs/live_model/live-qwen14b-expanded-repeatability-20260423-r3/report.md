# Rasputin Live-Model Replacement Benchmark Report

- Model: `qwen2.5-coder:14b`
- Tier: `expanded`
- Tasks: 5
- Pass: 5 (100%)
- Partial: 0 (0%)
- Fail: 0 (0%)
- Average recovery events: 3.6
- Average operator interventions: 0.0
- Verdict: **practical daily replacement**

## Task Results

| Task | Category | Result | Time | Validators | Trust |
|---|---|---:|---:|---:|---|
| `expanded_nested_module_feature` | Expanded: nested Rust module feature | PASS | 28.62s | 3/3 | audit=True, validation=True, runtime_error=False |
| `expanded_shared_helper_refactor` | Expanded: coordinated multi-file refactor | PASS | 18.82s | 6/6 | audit=True, validation=True, runtime_error=False |
| `expanded_bug_fix_cart` | Expanded: realistic failing-test bug fix | PASS | 19.38s | 1/1 | audit=True, validation=True, runtime_error=False |
| `expanded_onboarding_settings_change` | Expanded: repo onboarding find-and-change | PASS | 17.22s | 2/2 | audit=True, validation=True, runtime_error=False |
| `expanded_settings_vertical_slice` | Expanded: multi-surface vertical slice | PASS | 21.29s | 3/3 | audit=True, validation=True, runtime_error=False |

## Notes

- This benchmark runs the real Forge worker with live local-model planning.
- JSONL audit logs are preserved per task.
- Replay and TUI checkpoint continuity are marked `not_checked_worker_cli` in this worker-level harness and remain covered by deterministic TUI/runtime tests.
