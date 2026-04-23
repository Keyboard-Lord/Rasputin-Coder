# Rasputin Live-Model Replacement Benchmark Report

- Model: `qwen2.5-coder:14b`
- Tasks: 8
- Pass: 4 (50%)
- Partial: 4 (50%)
- Fail: 0 (0%)
- Average recovery events: 1.5
- Average operator interventions: 0.0
- Verdict: **usable with caveats**

## Task Results

| Task | Category | Result | Time | Validators | Trust |
|---|---|---:|---:|---:|---|
| `single_file_edit` | Single-file edit | PASS | 16.47s | 3/3 | audit=True, validation=True, runtime_error=False |
| `multi_file_feature` | Multi-file feature addition | PARTIAL | 59.7s | 1/3 | audit=True, validation=True, runtime_error=False |
| `refactor_with_validation` | Refactor with validation | PARTIAL | 21.79s | 1/2 | audit=True, validation=False, runtime_error=True |
| `bug_fix_from_failing_test` | Bug fix from failing test | PASS | 28.5s | 2/2 | audit=True, validation=True, runtime_error=False |
| `seeded_compile_error_recovery` | Seeded compile error recovery | PARTIAL | 49.43s | 0/2 | audit=True, validation=True, runtime_error=False |
| `seeded_validation_failure_recovery` | Seeded validation failure recovery | PASS | 17.15s | 2/2 | audit=True, validation=True, runtime_error=False |
| `repo_onboarding_find_change` | Repo onboarding / find-and-change task | PASS | 13.6s | 2/2 | audit=True, validation=True, runtime_error=False |
| `vertical_slice` | Small end-to-end vertical slice | PARTIAL | 82.42s | 0/3 | audit=True, validation=True, runtime_error=False |

## Notes

- This benchmark runs the real Forge worker with live local-model planning.
- JSONL audit logs are preserved per task.
- Replay and TUI checkpoint continuity are marked `not_checked_worker_cli` in this worker-level harness and remain covered by deterministic TUI/runtime tests.
