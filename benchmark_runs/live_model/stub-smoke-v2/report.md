# Rasputin Live-Model Replacement Benchmark Report

- Model: `qwen2.5-coder:14b`
- Tasks: 1
- Pass: 0 (0%)
- Partial: 1 (100%)
- Fail: 0 (0%)
- Average recovery events: 0.0
- Average operator interventions: 0.0
- Verdict: **usable with caveats**

## Task Results

| Task | Category | Result | Time | Validators | Trust |
|---|---|---:|---:|---:|---|
| `single_file_edit` | Single-file edit | PARTIAL | 0.44s | 0/3 | audit=True, validation=True, runtime_error=False |

## Notes

- This benchmark runs the real Forge worker with live local-model planning.
- JSONL audit logs are preserved per task.
- Replay and TUI checkpoint continuity are marked `not_checked_worker_cli` in this worker-level harness and remain covered by deterministic TUI/runtime tests.
