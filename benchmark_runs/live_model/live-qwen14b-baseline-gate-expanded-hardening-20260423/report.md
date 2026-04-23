# Rasputin Live-Model Replacement Benchmark Report

- Model: `qwen2.5-coder:14b`
- Tier: `baseline`
- Tasks: 8
- Pass: 8 (100%)
- Partial: 0 (0%)
- Fail: 0 (0%)
- Average recovery events: 1.12
- Average operator interventions: 0.0
- Verdict: **practical daily replacement**

## Task Results

| Task | Category | Result | Time | Validators | Trust |
|---|---|---:|---:|---:|---|
| `single_file_edit` | Single-file edit | PASS | 27.81s | 3/3 | audit=True, validation=True, runtime_error=False |
| `multi_file_feature` | Multi-file feature addition | PASS | 28.26s | 3/3 | audit=True, validation=True, runtime_error=False |
| `refactor_with_validation` | Refactor with validation | PASS | 55.75s | 2/2 | audit=True, validation=True, runtime_error=False |
| `bug_fix_from_failing_test` | Bug fix from failing test | PASS | 32.14s | 2/2 | audit=True, validation=True, runtime_error=False |
| `seeded_compile_error_recovery` | Seeded compile error recovery | PASS | 23.1s | 2/2 | audit=True, validation=True, runtime_error=False |
| `seeded_validation_failure_recovery` | Seeded validation failure recovery | PASS | 21.77s | 2/2 | audit=True, validation=True, runtime_error=False |
| `repo_onboarding_find_change` | Repo onboarding / find-and-change task | PASS | 17.04s | 2/2 | audit=True, validation=True, runtime_error=False |
| `vertical_slice` | Small end-to-end vertical slice | PASS | 30.09s | 3/3 | audit=True, validation=True, runtime_error=False |

## Notes

- This benchmark runs the real Forge worker with live local-model planning.
- JSONL audit logs are preserved per task.
- Replay and TUI checkpoint continuity are marked `not_checked_worker_cli` in this worker-level harness and remain covered by deterministic TUI/runtime tests.
