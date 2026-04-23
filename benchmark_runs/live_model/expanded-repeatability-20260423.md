# Expanded-Tier Repeatability Report - 2026-04-23

## Scope

- Model: `qwen2.5-coder:14b`
- Baseline gate: frozen `baseline` tier
- Expanded gate candidate: `expanded` tier
- Runs: 1 frozen baseline run, 5 expanded repeatability runs
- Scoring: unchanged benchmark scoring from `scripts/live_model_benchmark.py`

## Baseline Gate

| Run | Pass | Partial | Fail | Runtime Errors | Avg Recovery | Operator Interventions |
|---|---:|---:|---:|---:|---:|---:|
| `live-qwen14b-baseline-repeatability-gate-20260423` | 8 | 0 | 0 | 0 | 1.12 | 0.0 |

Baseline remains green at 8/8 PASS and stays the frozen replacement gate.

## Expanded Repeatability Runs

| Run | Pass | Partial | Fail | Runtime Errors | Avg Recovery | Operator Interventions |
|---|---:|---:|---:|---:|---:|---:|
| `live-qwen14b-expanded-repeatability-20260423-r1` | 5 | 0 | 0 | 0 | 3.6 | 0.0 |
| `live-qwen14b-expanded-repeatability-20260423-r2` | 5 | 0 | 0 | 0 | 3.6 | 0.0 |
| `live-qwen14b-expanded-repeatability-20260423-r3` | 5 | 0 | 0 | 0 | 3.6 | 0.0 |
| `live-qwen14b-expanded-repeatability-20260423-r4` | 5 | 0 | 0 | 0 | 3.6 | 0.0 |
| `live-qwen14b-expanded-repeatability-20260423-r5` | 5 | 0 | 0 | 0 | 3.6 | 0.0 |

## Variance

- Pass-rate mean: 100%
- Pass-rate population standard deviation: 0.0
- Average recovery-events mean: 3.6
- Average recovery-events population standard deviation: 0.0
- Runtime errors across expanded repeatability: 0/25 tasks
- Operator interventions across expanded repeatability: 0
- No-progress halt signatures: 0
- Audit log complete: 25/25 tasks
- Validation observed: 25/25 tasks

## Per-Task Stability

| Task | Results Across 5 Runs | Mean Time | Time Range | Recovery Events | Runtime Errors |
|---|---|---:|---:|---|---:|
| `expanded_nested_module_feature` | PASS x5 | 28.92s | 28.37-29.92s | 5,5,5,5,5 | 0 |
| `expanded_shared_helper_refactor` | PASS x5 | 18.89s | 18.78-19.19s | 6,6,6,6,6 | 0 |
| `expanded_bug_fix_cart` | PASS x5 | 19.18s | 18.83-19.41s | 3,3,3,3,3 | 0 |
| `expanded_onboarding_settings_change` | PASS x5 | 14.94s | 14.25-17.22s | 1,1,1,1,1 | 0 |
| `expanded_settings_vertical_slice` | PASS x5 | 21.43s | 21.29-21.65s | 3,3,3,3,3 | 0 |

## Promotion Decision

Promote expanded tier to stable second replacement gate.

The promotion criteria are satisfied:

- repeated runs stayed at full PASS
- no truth/runtime violations appeared in worker-level logs
- no operator interventions were required
- no task showed classification instability
- recovery behavior was stable run-to-run

Replay and checkpoint continuity remain covered by deterministic runtime/TUI tests; the live benchmark harness marks those worker-cli dimensions as `not_checked_worker_cli`.

## Next Gate

Preserve both gates:

1. `baseline` remains the frozen replacement gate.
2. `expanded` becomes the stable second replacement gate.

Future work should add a third, broader tier rather than modifying either promoted gate.
