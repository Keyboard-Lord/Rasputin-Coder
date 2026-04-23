# Rasputin Live-Model Replacement Benchmark Report

- Model: `qwen2.5-coder:14b`
- Tasks: 8
- Pass: 0 (0%)
- Partial: 6 (75%)
- Fail: 2 (25%)
- Average recovery events: 1.5
- Average operator interventions: 0.0
- Verdict: **usable with caveats**

## Task Results

| Task | Category | Result | Time | Validators | Trust |
|---|---|---:|---:|---:|---|
| `single_file_edit` | Single-file edit | PARTIAL | 41.46s | 3/3 | audit=True, validation=True, runtime_error=False |
| `multi_file_feature` | Multi-file feature addition | PARTIAL | 56.11s | 0/3 | audit=True, validation=True, runtime_error=False |
| `refactor_with_validation` | Refactor with validation | PARTIAL | 22.79s | 1/2 | audit=True, validation=False, runtime_error=True |
| `bug_fix_from_failing_test` | Bug fix from failing test | FAIL | 27.08s | 0/2 | audit=True, validation=False, runtime_error=False |
| `seeded_compile_error_recovery` | Seeded compile error recovery | PARTIAL | 50.14s | 0/2 | audit=True, validation=True, runtime_error=False |
| `seeded_validation_failure_recovery` | Seeded validation failure recovery | PARTIAL | 50.44s | 2/2 | audit=True, validation=True, runtime_error=False |
| `repo_onboarding_find_change` | Repo onboarding / find-and-change task | FAIL | 35.73s | 0/2 | audit=True, validation=False, runtime_error=False |
| `vertical_slice` | Small end-to-end vertical slice | PARTIAL | 79.5s | 0/3 | audit=True, validation=True, runtime_error=False |

## Notes

- This benchmark runs the real Forge worker with live local-model planning.
- JSONL audit logs are preserved per task.
- Replay and TUI checkpoint continuity are marked `not_checked_worker_cli` in this worker-level harness and remain covered by deterministic TUI/runtime tests.
