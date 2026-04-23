# Rasputin Future Roadmap and Extensibility

## Strategic Thesis

Rasputin is evolving from a single-run deterministic execution shell into a **controlled autonomous coding system**. This is not a rejection of safety constraints—it is an expansion of capability within them.

The roadmap prioritizes:
1. **Bounded autonomy**: Multi-step execution that remains interruptible and inspectable
2. **Controlled discovery**: Repository awareness without unbounded exploration
3. **Structured continuity**: State persistence that maintains validation boundaries
4. **Supervised execution**: Optional human checkpoints that fail closed
5. **Hard invariant preservation**: Local-first, validation-gated, bounded execution remain fixed

This document defines the phased path from current single-step execution to controlled agent behavior.

## Non-Negotiable Invariants

Regardless of autonomy expansion, the following constraints are **permanent**:

### Local-First Operation
- Ollama API on localhost only as core dependency
- No required external API calls (OpenAI, Anthropic, etc.)
- Optional remote Ollama may be supported, but never required
- No telemetry or analytics transmission

### Validation-Gated Persistence
- No file mutation without passing validation (Syntax → Build → Test)
- Auto-revert on validation failure (default-on, configurable but not disableable)
- No bypass mechanisms for "quick fixes"
- All mutations recorded with before/after hashes

### Bounded Execution
- Hard iteration limits per step (default 10, ceiling configurable)
- Hard chain length limits (TBD, but finite and enforced)
- Timeout bounds on all external operations
- No infinite loops, no unbounded recursion

### Explicit Auditability
- Every tool call logged with full arguments
- Chain state human-inspectable (structured JSON, not opaque binary)
- Complete execution history reconstructible from logs
- Deterministic context assembly (same input → same context)

### No Silent Background Mutation
- No daemon mode
- No background execution without an explicit user or goal trigger
- No scheduled tasks, no "watch mode" that auto-runs
- Execution tied to explicit user input: task-like plain text, `/goal`, `/task`, or equivalent command

These invariants define the boundary of acceptable implementation. Autonomy expansion happens **within** these constraints, not by removing them.

## Roadmap: Controlled Autonomy Under Hard Constraints

### Phase 1 — Read-Only Discovery Expansion

**Objective**: Enable bounded repository exploration without violating safety invariants.

**Status**: Partially complete as of [DATE].

**Completed**:
- ✅ `list_dir` and `grep_search` now planner-visible with bounds enforcement
- ✅ Discovery tools integrated into planner context

**Remaining**:
- `find_files` (planner-visible) - Glob-based file discovery | Max results 100
- Bounded repo scanning - Deterministic traversal | Max files 1000
- Automatic discovery on task start (opt-in)

**Deliverables**:

| Feature | Status | Description | Safety Constraint |
|---------|--------|-------------|-------------------|
| `list_dir` | ✅ Complete | Bounded directory listing | Max depth 5, max entries 1000 |
| `grep_search` | ✅ Complete | Pattern search with result limits | Max results 100 |
| `find_files` | Pending | Glob-based file discovery | Max results 100, no recursive globs |
| Bounded repo scanning | Pending | Deterministic traversal with explicit bounds | Max files scanned 1000 |
| Deterministic context assembly | ✅ Complete | Same discovery query → same context | Alphabetical ordering |

**Key Design Decision**: Discovery tools are **read-only** and **explicitly bounded**. They pre-populate context but do not bypass the read-before-write gate for mutations.

**Implementation Notes**:
- Execution-mode gating: Discovery only in `Analysis` mode
- Token budgeting: Discovery results count toward context budget
- Determinism: Traversal order fixed (alphabetical, depth-first)
- Caching: Results cached per task with TTL

---

### Phase 2 — Task Chaining (v1.0 Phase B/C: COMPLETE)

**Objective**: Enable multi-step objectives with structured state transfer between validated steps.

**Status**: Chain persistence, command surface, step tracking, and auto-resume implemented.

**Completed (Phase B)**:
- ✅ `PersistentChain` with full lifecycle (Draft→Ready→Running→Complete/Failed/Halted/Archived)
- ✅ `PersistentChainStep` with status tracking
- ✅ Chain persistence across restarts via `PersistentState.chains`
- ✅ **Command surface**: `/chains`, `/chain status`, `/chain switch`, `/chain archive`, `/chain resume`, `/plan`
- ✅ `ChainPolicy` with bounded execution constraints
- ✅ Conversation-to-chain binding

**Remaining**:
- Task-like plain text and `/goal` materialize planned chains; `/task` remains a manual Forge entrypoint
- Step result tracking (updating step after Forge completion)
- Automatic step advancement
- Input/output contracts between steps (type checking)

**Deliverables**:

| Feature | Status | Description | Safety Constraint |
|---------|--------|-------------|-------------------|
| Chain structure | ✅ Complete | `PersistentChain` primitive | Max chain length 100 (policy) |
| Step status | ✅ Complete | `PersistentChainStep` with lifecycle | Status: Pending/Running/Completed/Failed/Blocked |
| Persistence | ✅ Complete | Chain state survives restart | JSON in `~/.local/share/rasputin/state.json` |
| Command surface | ✅ Complete | `/chain` commands | All commands with real state mutations |
| Policy enforcement | ✅ Complete | `ChainPolicy` with bounds | max_steps, halt_on_failure, require_validation |
| Step result tracking | ✅ Complete | Update step after Forge completion | Wired via handle_chain_step_completion |
| Step advancement | ✅ Complete | Auto-advance on completion | Auto-resume triggers next step |
| Input/output contracts | ⏳ Pending | Type-checked contracts | Not yet implemented |

**Chain State Schema (Current)**:
```rust
pub struct PersistentChain {
    pub id: String,
    pub name: String,
    pub objective: String,
    pub status: ChainLifecycleStatus,  // Draft, Ready, Running, Halted, Failed, Complete, Archived
    pub steps: Vec<PersistentChainStep>,
    pub active_step: Option<usize>,
    pub repo_path: Option<String>,
    pub conversation_id: Option<String>,
    pub created_at: DateTime<Local>,
    pub updated_at: DateTime<Local>,
    pub completed_at: Option<DateTime<Local>>,
    pub archived: bool,
    pub total_steps_executed: u32,
    pub total_steps_failed: u32,
}

pub struct PersistentChainStep {
    pub id: String,
    pub description: String,
    pub status: ChainStepStatus,  // Pending, Running, Completed, Failed, Blocked
    pub tool_calls: Vec<String>,
    pub result_summary: Option<String>,
    pub validation_passed: Option<bool>,
    pub started_at: Option<DateTime<Local>>,
    pub completed_at: Option<DateTime<Local>>,
    pub error_message: Option<String>,
}
```

**Safety Mechanism**: Each step runs in its own Forge worker process. TUI maintains persistent chain orchestration state. No shared memory between steps.

---

### Phase 3 — Resumable Execution with Validated Checkpoints (Phase D: COMPLETE)

**Objective**: Enable pause/resume of chains with validated checkpoint preservation and workspace integrity verification.

**Status**: Fully implemented. Checkpoint system provides durable, audit-grounded snapshots with Blake3 workspace hashing and fail-closed resume validation.

**Completed (Phase D)**:
- ✅ `CheckpointManager` with durable per-chain checkpoint storage
- ✅ `ExecutionCheckpoint` with workspace hash, audit cursor, and validation status
- ✅ Blake3 workspace hash calculation for tracked files
- ✅ `/chain resume` with full checkpoint validation (schema, hash, audit cursor, replay state)
- ✅ Checkpoint creation at safe boundaries (validated step completion)
- ✅ Fail-closed resume: stale workspace → block, corrupted checkpoint → block, audit divergence → block
- ✅ `/checkpoint list` and `/checkpoints` commands for checkpoint inspection
- ✅ `CheckpointResumeResult` with explicit recovery guidance

**Deliverables**:

| Feature | Status | Description | Safety Constraint |
|---------|--------|-------------|-------------------|
| TUI persistence | ✅ Complete | `PersistentState.chains` survives restart | JSON in main state file |
| Resume command | ✅ Complete | `/chain resume` with checkpoint validation | Enforces max_steps, halt_on_failure |
| Autonomous continuation | ✅ Complete | Auto-resume after step completion | handle_chain_step_completion + try_auto_resume_chain |
| Checkpoint persistence | ✅ Complete | `~/.local/share/rasputin/chains/{id}/checkpoints/` | Only post-validation checkpoints |
| Resume validation | ✅ Complete | Workspace hash + audit cursor + replay state | Fail-closed on any mismatch |
| Stale-state detection | ✅ Complete | Blake3 hash verification | Detects any file changes |
| Checkpoint inspection | ✅ Complete | `/checkpoint list`, checkpoint metadata | JSON format, operator-facing |

**Checkpoint Policy (Implemented)**:
- Checkpoints created only after successful validated step completion
- Each checkpoint includes: chain state, audit cursor, workspace hash (Blake3), tracked files
- Resume validates: schema version, workspace hash, audit cursor consistency, replay state match
- Stale workspace → explicit operator review required (or `/chain resume --force`)
- Corrupted checkpoint → resume blocked with error message
- Audit divergence → resume blocked with divergence details

**Storage Format (Implemented)**:
```
~/.local/share/rasputin/chains/
  └── {chain_id}/
      └── checkpoints/
          └── chk-{chain_id}-{uuid}.json
```

---

### Phase 4 — Deterministic Audit and Replay (V1.6: COMPLETE)

**Objective**: Provide immutable execution timeline and deterministic reconstruction for debugging, forensics, and trust validation.

**Status**: Fully implemented. System operates on 5 layers of authoritative truth.

**Completed (V1.6)**:
- ✅ `AuditLog` with append-only `Vec<AuditEvent>`
- ✅ `AuditEventType` enum covering all state transitions and outcomes
- ✅ `AuditEvent` with timestamp, states, trigger, reason, step/task context
- ✅ `replay_audit_log()` deterministic reconstruction engine
- ✅ `ReplayResult` with final state, outcome, transitions, warnings
- ✅ `ReplayWarning` for inconsistency detection (MissingOutcome, InconsistentTransition, etc.)
- ✅ `/audit` command and Audit inspector tab
- ✅ `/audit replay` and `/chain replay` commands
- ✅ Replay validation against stored state/outcome
- ✅ 19 replay-specific unit tests

**System Truth Layers (V1.6)**:

| Layer | Type | Source | Purpose |
|-------|------|--------|---------|
| L1: Outcome | `ExecutionOutcome` | Chain aggregation | Terminal success/failure/warning |
| L2: Progress | `ExecutionState` | Reducer transitions | In-flight execution progress |
| L3: Audit | `AuditLog` | Runtime events | Immutable historical timeline |
| L4: Replay | `ReplayResult` | `replay_audit_log()` | Deterministic reconstruction |
| L5: Checkpoint | `ExecutionCheckpoint` | Validated boundaries | Resumable execution state |

**Critical Invariant**: Each layer derives from the layer below. UI NEVER invents truth independently.

**Replay Rules**:
- Deterministic: same audit log → same result
- Read-only: never mutates state or executes code
- No runtime dependency: uses only structured audit entries
- Validates: detects inconsistencies, gaps, impossible sequences

---

### Phase 5 — Approval and Supervision (FUTURE)

**Objective**: Optional human checkpoints between chain steps with fail-closed semantics.

**Status**: Chain-level halt/resume and autonomous continuation implemented. Per-step approval checkpoints not yet implemented.

**Completed (Phase B/C)**:
- ✅ Chain-level halt/resume via `/chain resume` (chains can be Halted and resumed)
- ✅ Chain status: WaitingForApproval (state exists, not yet triggered)
- ✅ Policy-level `require_approval_after_step_count` (field exists, not enforced)
- ✅ Autonomous continuation with operator override (`/chain archive`, `/chain switch` halt auto-resume)

**Current Gap**: Per-step approval not yet implemented. Within a chain step, execution proceeds atomically. Auto-resume provides step-level autonomy but not mid-step approval.

**Deliverables**:

| Feature | Description | Safety Constraint |
|---------|-------------|-------------------|
| Approval checkpoints | Explicit pause points in chain structure | User-configurable per step |
| Fail-closed pause | Chain halts if user doesn't respond | No timeout-based auto-continue |
| State display | Show pending changes before approval | Diff preview, validation status |
| Approval actions | Approve, reject, modify, or abort | All actions logged |
| Resume validation | Re-validate state on resume | Detect external changes during pause |

**Checkpoint Configuration**:
```rust
pub struct ApprovalCheckpoint {
    pub step_id: String,
    pub requires_approval: bool,
    pub display_diff: bool,
    pub display_validation: bool,
    pub timeout_seconds: Option<u32>, // None = indefinite
    pub default_action: ApprovalAction, // Default: halt if timeout
}
```

**Design Principle**: Approval is **opt-in per step**, not global. Default execution remains direct. User explicitly marks steps requiring review.

---

### Phase 5 — Rich Repo Awareness

**Objective**: Deep Git integration for context-aware planning and safe experimentation.

**Current Gap**: No Git awareness in planning or execution.

**Deliverables**:

| Feature | Description | Safety Constraint |
|---------|-------------|-------------------|
| Git status grounding | Modified/staged files in planner context | Read-only Git data |
| Branch awareness | Current branch in execution context | No automatic branch ops |
| Commit history summarization | Recent commits for context | Max 10 commits, read-only |
| Diff-aware planning | Show planned changes vs current state | Pre-execution diff preview |
| Relevance-based file loading | Priority files based on Git status | Staged/modified files prioritized |

**Git Integration Phases**:

**Phase 6a — Basic (Near Term)**:
- Git status display in TUI
- Branch indicator
- Pre-task "clean worktree" check
- Modified files in snapshot

**Phase 6b — Full (Medium Term)**:
- Optional auto-commit after validation
- Branch creation for experiments (`/branch`)
- Commit message generation from task description
- Merge conflict detection

**Configuration**:
```yaml
git:
  enable_grounding: true                # Include Git state in context
  auto_commit: false                    # Explicit opt-in
  commit_message_template: "[forge] {objective}"
  require_clean_worktree: false         # Allow dirty worktree
  create_branch_per_chain: false        # Isolate chains in branches
```

**Safety**: All Git operations are explicit user actions or opt-in automation. No silent commits, no auto-push.

---

### Phase 7 — Advanced Bounded Autonomy

**Objective**: Sophisticated agent behaviors within hard execution constraints.

**Current Gap**: Single-objective, single-step execution only.

**Deliverables**:

| Feature | Description | Safety Constraint |
|---------|-------------|-------------------|
| Multi-step refactors | Cross-file coordinated changes | Each file validated independently |
| Guided fix loops | Automatic retry with adjusted parameters | Max 3 retries, then halt |
| Controlled batch edits | Same operation across multiple files | Max batch size 50 files |
| Safe mode escalation | Promote from Analysis → Edit → Fix | Explicit mode transitions only |
| Policy-driven execution | User-defined rules for auto-approval | Rules human-readable, auditable |

**Execution Mode Escalation**:
```
Analysis (read-only discovery)
         │
         │ explicit user trigger
         ▼
Edit (file mutations with validation)
         │
         │ validation failure + retry
         ▼
Fix (targeted error correction)
         │
         │ all modes bounded by iteration limits
         ▼
Batch (coordinated multi-file)
```

**Policy Engine** (Phase 6 advanced):
```yaml
execution_policy:
  auto_approve:
    - pattern: "test_*.rs"
      change_type: "modify"
      max_lines: 10
    - pattern: "*.md"
      change_type: "any"
  require_approval:
    - pattern: "src/main.rs"
      change_type: "modify"
  max_batch_size: 20
  max_retry_attempts: 3
```

**Invariant Preservation**: Even with advanced autonomy, all previous constraints hold:
- Each step validated independently
- Total iterations bounded across all retries
- All changes auditable and reversible
- No background execution

---

## Implementation Priorities

| Phase | Priority | Est. Timeline | Blockers |
|-------|----------|---------------|----------|
| 1 — Discovery | P1 (Critical) | 4-6 weeks | None |
| 2 — Chaining | P1 (Critical) | 6-8 weeks | Phase 1 |
| 3 — Resumable | P2 (High) | 4-6 weeks | Phase 2 |
| 4 — Approval | P2 (High) | 6-8 weeks | Phase 3 |
| 5 — Git | P2 (High) | 4-6 weeks | None |
| 6 — Advanced | P3 (Medium) | 8-12 weeks | Phases 1-5 |

**Critical Path**: Phase 1 (Discovery) unlocks Phase 2 (Chaining). Chaining unlocks Resumable and Approval. Git can proceed in parallel.

---

## Extensibility Mechanisms

### Tool Registration

**Current**: 7 tools registered, 3 planner-visible.

**Extension path for new tools**:
```rust
impl Tool for MyCustomTool {
    fn name(&self) -> ToolName { ToolName::new("my_tool").unwrap() }
    fn execute(&self, args: &ToolArguments, ctx: &ExecutionContext) -> Result<ToolResult, ForgeError> {
        // Implementation
    }
}

// In ToolRegistry::new():
Self::register_tool(&mut tools, MyCustomTool::new());
```

**Visibility Rules**:
- Read-only tools: Available in `Analysis` mode
- Mutation tools: Available in `Edit`, `Fix`, `Batch` modes
- Discovery tools: Phase 1 enables for planner

---

## Permanent Constraints (Will Not Change)

The following are **architectural commitments**, not temporary limitations:

| Constraint | Rationale |
|------------|-----------|
| **Local-first operation** | Privacy, offline capability, zero external cost are core value propositions |
| **Validation-gated persistence** | No "trust me" mode—every mutation proven safe before commit |
| **Bounded execution** | Infinite loops and unbounded recursion are categorically excluded |
| **Explicit auditability** | All state human-inspectable; no opaque binary blobs |
| **No silent background execution** | User intent must be explicit for every execution |

**Note on Autonomy**: Rasputin **will** implement bounded autonomy—task chaining, discovery, resumable execution. These are **not** rejected. They are being built **within** the constraints above.

What remains excluded:
- **Cloud AI as requirement**: Local models are mandatory; cloud is optional only
- **Unbounded background agents**: No daemon mode, no scheduled execution
- **Implicit execution**: All automation is explicit, inspectable, and interruptible

---

## Sprint v1.0 Progress (Consolidated)

**Status**: Phase A Complete, Pre-existing Errors Fixed, TUI Commands Added

### Completed Work
1. **Phase A - Core Persistence Models**: ✅ COMPLETE
   - `ChainRegistry` with `ChainLifecycleStatus`, `ChainRecord`
   - `SessionContinuityState` for operator context
   - `ContextAssemblyResult` with bounded file selection
   - `ChainPlanSummary` and `ChainContinuationSummary`

2. **Pre-existing Compilation Errors**: ✅ FIXED
   - approval_checkpoint.rs borrow checker fixes
   - task_intake.rs Display trait for ExecutionMode
   - observability.rs TimelinePhase variants
   - planner_attack_fixtures.rs ToolArguments usage
   - types.rs serde imports

3. **TUI Chain Commands**: ✅ ADDED
   - `/chains`, `/chain status`, `/chain switch`, `/chain archive`, `/chain resume`
   - Chain persistence across restarts
   - Auto-resume with policy bounds

## Contribution Areas (Updated for V1.5)

### Completed in V1.5

| Area | Status | Description |
|------|--------|-------------|
| **Chain persistence** | ✅ Complete | JSON serialization for chains with hash verification |
| **Git integration** | ✅ Complete | Git grounding with dirty worktree detection |
| **Risk forecasting** | ✅ Complete | Critical risk detection and blocking |
| **Interrupt handling** | ✅ Complete | /stop with context preservation |
| **Auto-resume** | ✅ Complete | Policy-gated autonomous continuation |

### Remaining Work

| Area | Description | Skills |
|------|-------------|--------|
| **Lint stage** | Project-type detection + standard lint commands | Rust, build systems |
| **Bounded discovery** | Expand list_dir, grep_search capabilities | Rust, tree traversal |
| **Context assembly** | Import graph analysis for relevance ranking | Rust, parsing |
| **Policy engine** | YAML-defined auto-approval rules | Rust, config |
| **Checkpoint wiring** | Wire approval checkpoints to hot path | Rust, ratatui |
| **Testing** | Automated tests for chain flows | Rust, testing |

### Research Directions

| Area | Description |
|------|-------------|
| **Determinism** | Quantization variance characterization, reproducibility metrics |
| **Planner optimization** | Few-shot prompting, tool use patterns, failure recovery |
| **RLEF** | Reinforcement learning from execution feedback for hint generation |

---

## Technical Debt

| Issue | Location | Priority | Notes |
|-------|----------|----------|-------|
| Interface layer | `crates/rasputin-interface/` | Medium | Promote to hot path or remove |
| Error consolidation | `types.rs` across crates | Low | Unify error types |
| Validation extensibility | `validation_engine.rs` | Medium | Plugin architecture for custom validators |
| TUI state | `apps/rasputin-tui/src/state.rs` | Low | Normalize state management |

---

## Success Metrics

| Metric | Current | Phase 2 Target | Phase 6 Target |
|--------|---------|----------------|----------------|
| Task success rate | ~70% | >80% | >90% |
| Multi-step completion | N/A | >70% chains complete | >85% chains complete |
| Validation pass rate | ~85% | >90% | >95% |
| User intervention rate | High | Medium (approval pts) | Low (policy-driven) |
| Mean time to completion | 5 min | 10 min (chains) | 15 min (complex refactors) |
| Safety incidents | 0 | 0 | 0 |

---

## Conclusion

Rasputin is becoming a **controlled autonomous coding system**—not by removing constraints, but by building sophisticated behavior within them.

The path forward is clear:
1. **Bounded discovery** enables context awareness without unbounded exploration
2. **Task chaining** enables multi-step objectives with per-step validation
3. **Resumable execution** enables long-running work with safety checkpoints
4. **Approval and supervision** enable trust through inspectability and control
5. **Git awareness** enables safe experimentation and collaboration
6. **Advanced autonomy** enables sophisticated behaviors with policy enforcement

All of this happens under the hard invariants: local-first, validation-gated, bounded, explicit, auditable.

The system that emerges will be capable of complex, multi-step software engineering tasks while remaining trustworthy, inspectable, and entirely under user control.
