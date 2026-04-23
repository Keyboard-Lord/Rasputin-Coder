# Rasputin Live-Model Replacement Benchmark Report

- Model: `qwen2.5-coder:14b`
- Tasks: 8
- Pass: 0 (0%)
- Partial: 2 (25%)
- Fail: 6 (75%)
- Average recovery events: 0.0
- Average operator interventions: 0.0
- Verdict: **not ready**

## Task Results

| Task | Category | Result | Time | Validators | Trust |
|---|---|---:|---:|---:|---|
| `single_file_edit` | Single-file edit | FAIL | 28.14s | 0/3 | audit=True, validation=False, runtime_error=False |
| `multi_file_feature` | Multi-file feature addition | PARTIAL | 34.22s | 0/3 | audit=True, validation=True, runtime_error=True |
| `refactor_with_validation` | Refactor with validation | PARTIAL | 30.47s | 1/2 | audit=True, validation=False, runtime_error=False |
| `bug_fix_from_failing_test` | Bug fix from failing test | FAIL | 26.89s | 0/2 | audit=True, validation=False, runtime_error=False |
| `seeded_compile_error_recovery` | Seeded compile error recovery | FAIL | 28.23s | 0/2 | audit=True, validation=False, runtime_error=False |
| `seeded_validation_failure_recovery` | Seeded validation failure recovery | FAIL | 35.74s | 0/2 | audit=True, validation=False, runtime_error=False |
| `repo_onboarding_find_change` | Repo onboarding / find-and-change task | FAIL | 35.47s | 0/2 | audit=True, validation=False, runtime_error=False |
| `vertical_slice` | Small end-to-end vertical slice | FAIL | 35.75s | 0/3 | audit=True, validation=False, runtime_error=False |

## Notes

- This benchmark runs the real Forge worker with live local-model planning.
- JSONL audit logs are preserved per task.
- Replay and TUI checkpoint continuity are marked `not_checked_worker_cli` in this worker-level harness and remain covered by deterministic TUI/runtime tests.
