# Rasputin Repeatability And Scope Report - 2026-04-23

## Baseline Gate

Model: `qwen2.5-coder:14b`

Baseline corpus: 8 tasks

Stability threshold for replacement confidence:

- PASS rate must be at least 95% across the repeated baseline corpus.
- FAIL count must remain 0.
- Operator interventions must remain 0.
- Runtime errors must remain 0 for tasks counted as PASS.
- Any PARTIAL must be explained as either benchmark scoring mismatch or an isolated hardening target before scope expansion.

## Baseline Runs

| Run | Report | Pass | Partial | Fail | Avg Recovery Events | Operator Interventions | Notes |
|---|---|---:|---:|---:|---:|---:|---|
| r1 | `benchmark_runs/live_model/live-qwen14b-repeatability-20260423-r1/report.md` | 7 | 1 | 0 | 1.12 | 0.0 | Refactor task used a valid private helper named `format_prefix`; old validator required exact name `format_with_prefix`. |
| r2 | `benchmark_runs/live_model/live-qwen14b-repeatability-20260423-r2/report.md` | 7 | 1 | 0 | 1.25 | 0.0 | Same scoring issue as r1. Runtime completed, validation passed, audit was complete. |
| r3 | `benchmark_runs/live_model/live-qwen14b-repeatability-20260423-r3/report.md` | 8 | 0 | 0 | 0.75 | 0.0 | Clean 8/8. |
| r4 corrected | `benchmark_runs/live_model/live-qwen14b-repeatability-20260423-r4-corrected/report.md` | 8 | 0 | 0 | 0.75 | 0.0 | Fresh run after replacing the over-specific refactor validator with semantic validation. |

Corrected semantic validator result for r1, r2, and r3 refactor artifacts: PASS.

Corrected baseline stability verdict: stable for the current benchmark corpus.

## Harness Changes

- Baseline remains the default tier: `--tier baseline`.
- Expanded tasks are isolated behind `--tier expanded`.
- The refactor validator now checks the objective semantics:
  - public `label_user` and `label_team` remain present,
  - direct repeated `format!("user:{}"` / `format!("team:{}"` calls are removed,
  - a private helper accepts `prefix` and `name`,
  - both label functions call the helper.

## Expanded Tier

Expanded corpus: 5 tasks

Run: `benchmark_runs/live_model/live-qwen14b-expanded-scope-20260423-r1/report.md`

Result: 1 PASS / 2 PARTIAL / 2 FAIL

| Task | Result | Validators | Primary Bottleneck |
|---|---:|---:|---|
| `expanded_nested_module_feature` | PARTIAL | 1/3 | Nested module recovery found `src/auth/token.rs`, but recovery got stuck on read-before-write and read-only churn after clippy evidence. |
| `expanded_shared_helper_refactor` | PARTIAL | 1/6 | Early read-only churn on coordinated multi-file refactor; no mutation reached the required helper module surfaces. |
| `expanded_bug_fix_cart` | FAIL | 0/1 | Early read-only churn before mutation on realistic failing-test bug fix. |
| `expanded_onboarding_settings_change` | PASS | 2/2 | Larger onboarding find-and-change path succeeded with format recovery and validated completion. |
| `expanded_settings_vertical_slice` | FAIL | 0/3 | Early read-only churn before mutation on multi-surface vertical slice. |

Expanded-corpus verdict: experimental, not replacement-grade for this broader scope yet.

## Validation

Deterministic validation after harness/corpus changes:

- `PYTHONPYCACHEPREFIX=/tmp/rasputin-pycache python3 -m py_compile scripts/live_model_benchmark.py`
- `cargo check -p forge_bootstrap`
- `cargo test -p forge_bootstrap`
- `cargo check -p rasputin-tui`
- `cargo test -p rasputin-tui`
- `cargo test`

All passed.

## Next Bottleneck

The next real bottleneck is expanded-task live convergence, specifically read-only churn on larger multi-file tasks and recovery follow-through after nested Rust module/clippy evidence. Baseline reliability should not be touched while addressing this tier.
