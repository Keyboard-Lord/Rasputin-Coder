# Live-Model Replacement Benchmark

This benchmark is a gated operator-run procedure for measuring whether local-model Rasputin can replace Codex for daily engineering work.

It is intentionally outside deterministic CI because it depends on:

- a live loopback Ollama server
- installed local models
- model output quality
- wall-clock runtime

Run:

```bash
python3 scripts/live_model_benchmark.py --model qwen2.5-coder:14b
```

Outputs are written under `benchmark_runs/live_model/<run-id>/`:

- `summary.json`
- `report.md`
- one workspace per task
- raw `stdout.jsonl` and `stderr.txt` per task

Scoring:

- `PASS`: objective completed, validators passed, runtime exited successfully, no truth violations detected by the harness
- `PARTIAL`: useful progress or some validators passed, but completion required intervention or remained incomplete
- `FAIL`: objective not completed, validation failed, runtime halted unrecovered, or final state was invalid

The benchmark records worker JSONL audit evidence directly. TUI checkpoint/replay continuity remains covered by deterministic tests and can be exercised manually after a benchmark run through `/checkpoint status`, `/chain status`, and `/audit replay`.
