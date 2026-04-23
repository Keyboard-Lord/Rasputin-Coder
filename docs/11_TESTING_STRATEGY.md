# Rasputin Testing Strategy

## Overview

The testing strategy focuses on:
- **Unit tests** for core types and logic
- **Integration tests** for validation engines
- **Conformance tests** for planner contracts
- **Manual testing** for TUI interactions

## Test Organization

### Package Structure

| Package | Test Location | Focus |
|---------|---------------|-------|
| `rasputin-tui` | `apps/rasputin-tui/src/` (inline) | State, persistence, chain management, commands |
| `forge_bootstrap` | `crates/forge-runtime/src/conformance_tests.rs` | Runtime, validation, chain execution, planner contracts |

## Running Tests

### All Tests
```bash
cargo test
```

### Package-Specific
```bash
cargo test -p rasputin-tui
cargo test -p forge_bootstrap
```

### Quiet Mode (CI)
```bash
cargo test -p forge_bootstrap --quiet
cargo test -p rasputin-tui --quiet
```

### With Output
```bash
cargo test -- --nocapture
```

## Test Categories

### 1. Unit Tests (Core Types)

Location: Inline in source files

**workspace_config.rs tests**:
```rust
#[test]
fn forge_yaml_takes_precedence_over_rasputin_json() { ... }

#[test]
fn detects_standard_git_repository() { ... }

#[test]
fn detects_git_worktree_file_reference() { ... }
```

These test config resolution and git detection.

### 2. V1.6 Truth Layer Tests (Critical)

Location: `apps/rasputin-tui/src/validation_tests.rs`

**State Reducer Tests**:
- `test_state_transitions_idle_to_done` - Canonical transition flow
- `test_terminal_outcome_is_sticky` - Terminal state stickiness
- `test_force_override_resets_terminal_state` - Override semantics
- `test_repair_success_path` - Repair transition validation
- `test_repair_failure_path` - Repair failure handling

**AuditLog Tests**:
- `test_audit_log_new_is_empty` - Empty log initialization
- `test_audit_log_append_increases_length` - Append semantics
- `test_audit_log_get_last_n_returns_most_recent` - Event ordering
- `test_audit_log_get_transition_history` - Transition filtering
- `test_audit_log_get_outcome_trace` - Outcome event filtering

**Replay Engine Tests**:
- `test_replay_empty_audit_log_returns_initial_state` - Empty replay
- `test_replay_applied_transitions_reconstructs_state` - State reconstruction
- `test_replay_normal_flow_idle_planning_executing_done` - Normal flow
- `test_replay_rejected_transitions_preserved` - Rejected transition tracking
- `test_replay_normalized_transitions_reconstructs_state` - Normalized handling
- `test_replay_outcome_finalization_parsed` - Outcome parsing
- `test_replay_success_with_warnings_outcome` - Warning outcome
- `test_replay_failed_outcome` - Failed outcome
- `test_replay_blocked_outcome` - Blocked outcome
- `test_replay_missing_outcome_warning_on_terminal_state` - Missing outcome detection
- `test_replay_multiple_outcomes_warning` - Multiple outcome detection
- `test_replay_inconsistent_transition_warning` - Inconsistency detection
- `test_replay_validation_matches_stored_state` - Validation success
- `test_replay_validation_detects_state_divergence` - State divergence
- `test_replay_validation_detects_outcome_divergence` - Outcome divergence
- `test_replay_summary_format` - Human-readable summary
- `test_replay_independent_of_live_runtime` - Runtime independence

**Checkpoint Tests**:
- `test_checkpoint_creation_basic` - Checkpoint structure
- `test_checkpoint_mark_valid` - Validation marking
- `test_checkpoint_mark_invalid` - Invalid marking
- `test_checkpoint_terminal_status_not_resumable` - Terminal handling
- `test_checkpoint_resume_result_variants` - Resume result types
- `test_checkpoint_validation_result_variants` - Validation results
- `test_checkpoint_filename_generation` - Filename format
- `test_chain_lifecycle_status_is_terminal` - Terminal detection
- `test_checkpoint_schema_version` - Schema versioning
- `test_checkpoint_source_variants` - Source types

### 3. Validation Engine Tests

**Syntax Validation**:
- Python: AST parsing
- JavaScript/TypeScript: ESLint-style checks (basic)
- Rust: `rustc --emit=metadata` or similar

**Build Validation**:
- Rust: `cargo check --quiet`
- TypeScript: `tsc --noEmit`
- Node: `npm run build --if-present`

**Test Validation**:
- Rust: `cargo test --quiet`
- Node: `npm test --if-present`
- Python: `python -m pytest -q`

### 3. Conformance Tests

Location: `crates/forge-runtime/src/conformance_tests.rs`

Tests planner output contract conformance:
- JSON schema validation
- Tool name existence
- Required arguments
- Mode restrictions
- Read-before-write satisfaction

### 4. Integration Tests

**End-to-End Trace**:
Located in `examples/end_to_end_trace/`:
- Sample execution traces
- JSONL event sequences
- Validation reports

## CI/CD Testing

GitHub Actions (`.github/workflows/ci.yml`):

```yaml
jobs:
  rust:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Test Forge runtime
        run: cargo test -p forge_bootstrap --quiet
      - name: Test Rasputin TUI
        run: cargo test -p rasputin-tui --quiet
```

## Manual Testing

### TUI Testing Checklist

1. **Startup**
   - [ ] `./rasputin` with workspace path
   - [ ] `./rasputin` without path (restore last)
   - [ ] Terminal profile installation (macOS)

2. **Chat**
   - [ ] Plain message sends to Ollama
   - [ ] Response displays in transcript
   - [ ] Blocked when no repo attached

3. **Commands**
   - [ ] `/open <path>` attaches repo
   - [ ] `/model` shows active model
   - [ ] `/models` lists available models
   - [ ] `/status` shows runtime state
   - [ ] `/validate` runs validation
   - [ ] `/help` displays help

4. **Forge Tasks**
   - [ ] `/task create hello.txt` executes
   - [ ] Inspector opens automatically
   - [ ] Runtime tab updates
   - [ ] Validation tab updates
   - [ ] Diff tab shows changes
   - [ ] `[done]` notice appears

5. **Persistence**
   - [ ] Conversations persist across restarts
   - [ ] Recent repos list updates
   - [ ] Model status persists

### Forge Testing Checklist

1. **Basic Task**
   ```bash
   cargo run -p forge_bootstrap -- "Create hello.txt with 'hello'" 10 http
   ```

2. **With Different Models**
   ```bash
   FORGE_PLANNER_MODEL=qwen2.5-coder:7b cargo run -p forge_bootstrap -- "task"
   ```

3. **JSONL Output**
   ```bash
   FORGE_OUTPUT_MODE=jsonl cargo run -p forge_bootstrap -- "task"
   ```

4. **Validation Stages**
   - Test with Rust project: syntax, build, test
   - Test with Python project: syntax, test
   - Test with JS project: syntax, build, test

### Gated Live-Model Replacement Benchmark

This benchmark is explicitly outside deterministic CI. It measures whether the
live local-model loop can complete representative daily SWE tasks through the
real Forge worker.

```bash
python3 scripts/live_model_benchmark.py --model qwen2.5-coder:14b
```

The corpus lives in `benchmarks/live_model/corpus.json`. Results are written to
`benchmark_runs/live_model/<run-id>/` with:

- raw JSONL worker audit logs
- per-task disposable workspaces
- independent validator results
- `summary.json`
- `report.md`

Scoring is PASS/PARTIAL/FAIL. The benchmark tracks runtime success, validation,
recovery events, operator interventions, and worker-level audit completeness.
Replay and TUI checkpoint continuity remain covered by deterministic TUI/runtime
tests and should be manually spot-checked from the TUI when validating a release
candidate.

## Test Patterns

### State Testing
```rust
#[test]
fn state_hash_verification() {
    let state = AgentState::new("task");
    let hash = state.compute_hash();
    assert!(state.verify_integrity(&hash));
}
```

### Tool Testing
```rust
#[test]
fn read_file_tool_returns_content() {
    let tool = ReadFileTool::new();
    let mut args = ToolArguments::new();
    args.set("path", "test.txt");
    let ctx = ExecutionContext::new();
    let result = tool.execute(&args, &ctx).unwrap();
    assert!(result.success);
}
```

### Validation Testing
```rust
#[test]
fn syntax_validation_fails_on_invalid_rust() {
    let code = "fn main { }";  // Missing parens
    let result = validate_rust_syntax(code);
    assert!(result.is_err());
}
```

## Coverage Goals

| Component | Target | Status |
|-----------|--------|--------|
| workspace_config.rs | High | ✅ Well-tested |
| Tool registry | Medium | ⚠️ Basic coverage |
| Validation engine | High | ⚠️ Project-dependent |
| Runtime loop | Medium | ⚠️ Integration tests |
| TUI state | Medium | ⚠️ Manual testing |
| Chain persistence | High | ✅ Phase B implemented |
| Chain policy | High | ✅ Phase B implemented |
| Auto-resume | High | ✅ V1.5 - implemented |
| Risk forecasting | Medium | 🔄 V1.5 - needs tests |
| Interrupt handling | High | ✅ V1.5 - implemented |
| Task intake | Medium | ✅ V1.5 - implemented |
| Git grounding | Medium | ✅ V1.5 - implemented |
| Approval checkpoints | Medium | 🔄 V1.5 - needs tests |

## Chain Testing (Phase C)

### Manual Chain Testing Checklist

```bash
# 1. Chain creation and persistence
/chains                              # Should show "No active chains"
/task "Create a hello world file"    # Creates implicit chain
/chains                              # Should show new chain with status
/quit                                # Exit Rasputin
# Restart Rasputin
/chains                              # Chain should still exist (survived restart)

# 2. Chain switching and binding
/chain switch <chain-id>             # Switch to specific chain
/chain status                         # Show active chain details
/chain status <other-id>             # Show specific chain details

# 3. Chain archival (fail-closed test)
/chain archive <running-chain-id>    # Should reject (cannot archive running)
# Wait for chain to complete
/chain archive <completed-chain-id> # Should succeed
/chains                              # Should not show archived chain

# 4. Chain resume flow
/chain resume <chain-id>             # Resume specific chain
/chain resume active                  # Resume using "active" keyword
/resume                              # Alias test
/continue                            # Alias test

# 5. Auto-resume flow (Phase C)
# Set policy.auto_resume = true (via code or config)
/chain resume <chain-id>             # Start chain
# Wait for step completion
# Chain should auto-resume to next step WITHOUT manual /chain resume
/chains                              # Verify chain still Running
/chain status                        # Verify step advanced
# Verify notifications show auto-resume triggered

# 6. Auto-resume safety tests (Phase C)
# Test: max_steps enforcement during auto-resume
# Test: halt_on_failure stops auto-resume
# Test: operator can /chain archive to halt auto-resume mid-chain

# 5. Plan inspection
/plan                                # Show steps for active chain
/plan checkpoints                    # Show checkpoint info
/plan context                        # Show context (V2 placeholder)

# 6. Risk preview (V1.5)
/preview                             # Preview chain with risk forecast
/chain resume <id>                  # Should show risk summary before execution
/chain resume <id> --force          # Bypass critical risks

# 7. Interrupt and resume (V1.5)
/chain resume <id>                  # Start chain
/stop                                # Interrupt execution
/chain status                        # Verify step shows Failed
/chain resume <id>                  # Resume from interruption

# 8. Git grounding (V1.5)
/task "modify file"                  # With dirty worktree
# Should warn about uncommitted changes
```

### Chain Unit Tests Needed

```rust
#[test]
fn chain_creation_sets_active_and_binds_to_conversation() {
    let mut state = PersistentState::new();
    let chain = state.create_chain("Test Chain", "Test objective");
    assert_eq!(state.active_chain_id, Some(chain.id.clone()));
    // Verify conversation binding if active_conversation exists
}

#[test]
fn chain_archive_fails_if_running() {
    let mut state = PersistentState::new();
    let chain = state.create_chain("Test", "Objective");
    // Set chain status to Running
    state.get_chain_mut(&chain.id).unwrap().status = ChainLifecycleStatus::Running;
    
    let result = state.archive_chain(&chain.id);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("running"));
}

#[test]
fn chain_resume_respects_max_steps_policy() {
    let mut state = PersistentState::new();
    state.chain_policy.max_steps = 5;
    let chain = state.create_chain("Test", "Objective");
    // Set total_steps_executed to 5 (at limit)
    state.get_chain_mut(&chain.id).unwrap().total_steps_executed = 5;
    
    // Attempting to resume should fail policy check
}

#[test]
fn chain_persists_across_serialization() {
    let mut state = PersistentState::new();
    let chain = state.create_chain("Test", "Objective").clone();
    
    let json = serde_json::to_string(&state).unwrap();
    let restored: PersistentState = serde_json::from_str(&json).unwrap();
    
    assert_eq!(restored.chains.len(), 1);
    assert_eq!(restored.chains[0].id, chain.id);
    assert_eq!(restored.active_chain_id, Some(chain.id));
}
```

## Known Testing Gaps

1. **TUI rendering**: Manual testing only; no automated UI tests
2. **Ollama integration**: Requires running Ollama; mocked in some tests
3. **Full execution flows**: Relies on manual end-to-end testing
4. **Cross-platform**: CI only tests Ubuntu; macOS/Windows manually tested
5. **Chain persistence**: Needs automated restart survival tests
6. **Chain policy enforcement**: Needs policy violation tests
7. **Chain result tracking**: Needs step completion/update tests
8. **Auto-resume**: Needs automated multi-step chain flow tests
9. **Bounded autonomy**: Needs max_steps enforcement during auto-resume tests

## V1.5 P0 Validation Matrix (Consolidated)

### TEST 1: /stop During Active Execution
**Objective:** Verify /stop actually kills worker and leaves coherent blocked state

**Steps:**
1. Start execution: `/plan test task` → `/chain resume`
2. While Running: `/stop`
3. Verify: Process killed, chain status = Halted, interrupt_context captured

**Expected State:**
- `active_execution_runtime` = None
- `state.execution.state` = Blocked
- `interrupt_context` = Some (with correct step)
- Chain status = Halted (not Running)

---

### TEST 2: GitConflict Blocks Without --force
**Objective:** Verify GitConflict risks block execution unless explicitly overridden

**Steps:**
1. Modify a file in repo (don't commit)
2. `/plan modify that file` (creates write step)
3. `/chain resume` (no --force)

**Expected Behavior:**
- ❌ Execution blocked
- Message: "Critical risks detected - execution blocked"
- Shows: GitConflict with affected file
- Suggests: /preview, /chain resume --force

---

### TEST 3: GitConflict --force Override
**Objective:** Verify --force flag allows explicit override

**Steps:**
1. Same setup as TEST 2 (dirty git + write step)
2. `/chain resume --force`

**Expected Behavior:**
- ⚠️ Warning shown: "Force override active - proceeding despite critical risks"
- ▶ Execution proceeds

---

### TEST 4: Completion Clears pending_confirmation
**Objective:** Verify no stale pending action after chain completion

**Steps:**
1. Create plan with confirmation-prone step
2. Execute to completion (all steps done)
3. Check: `pending_confirmation` is None

---

### TEST 5: Interrupt Context Captures Correct Step
**Objective:** Verify step number is accurate in interrupt context

**Steps:**
1. Start multi-step chain
2. Let it progress to step 2+
3. `/stop` during step execution

**Expected Context:**
```
Execution paused.
You were:
  Step 3 of 5 (Actual step description)
→ Resume: /chain resume
→ Inspect: /replay diff 3
```

## Regression Testing

Before releases, verify:
1. All unit tests pass
2. Manual TUI checklist completes
3. Sample tasks succeed with recommended model
4. Persistence works across restarts
5. **Chain persistence works across restarts** (V1.5)
6. **Chain commands function correctly** (V1.5)
7. **Auto-resume advances chains** (V1.5) - with auto_resume enabled
8. **Policy bounds stop auto-resume** (V1.5) - max_steps, halt_on_failure
9. **Risk preview blocks on critical risks** (V1.5)
10. **Interrupt handling preserves chain state** (V1.5)
11. **Git grounding detects dirty worktree** (V1.5)
12. **V1.5 P0 Validation Matrix passes** (all 5 tests above)
13. Build passes with `--release` profile
