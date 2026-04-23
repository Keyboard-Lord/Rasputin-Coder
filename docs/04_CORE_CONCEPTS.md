# Rasputin Core Concepts

## System Truth Layers (V1.6)

Rasputin implements a hierarchy of five authoritative truth layers. Each layer derives from the layer below. **UI NEVER invents truth independently.**

| Layer | Type | Source | Immutable | Purpose |
|-------|------|--------|-----------|---------|
| **L1: Outcome** | `ExecutionOutcome` | Chain aggregation | Yes (finalized) | Terminal success/failure/warning |
| **L2: Progress** | `ExecutionState` | Reducer transitions | No (transient) | In-flight execution progress |
| **L3: Audit** | `AuditLog` | Runtime events | Yes (append-only) | Historical execution timeline |
| **L4: Replay** | `ReplayResult` | `replay_audit_log()` | Yes (deterministic) | Reconstructable state validation |
| **L5: Checkpoint** | `ExecutionCheckpoint` | Validated boundaries | Yes (snapshots) | Resumable execution state |

**Flow**: Runtime events → AuditLog → Replay validation → Checkpoint snapshots → Resume

## Domain Model

### Session Types

| Concept | Type | Description |
|---------|------|-------------|
| **Rasputin Session** | Long-lived | Product session with chat history, repos, settings |
| **Forge Session** | Per-task | Worker session for single bounded execution |
| **Conversation** | Persistent | Chat thread with messages, archived status |
| **Task** | Bounded | One Forge execution with max iterations |
| **Chain** | Persistent | Multi-step objective with step tracking |

### Core State Types

#### ExecutionOutcome (Layer 1 - Terminal Truth)
Single authoritative terminal state per chain:

```rust
enum ExecutionOutcome {
    Success,              // All steps completed, no warnings
    SuccessWithWarnings,  // Completed with non-fatal warnings
    Failed,               // Step failed, halted
    Blocked,              // Blocked by approval denial or system issue
}
```

**Rules**:
- One outcome per chain, set at completion
- Aggregated from step outcomes
- Immutable once finalized
- Source of truth for UI status

#### ExecutionState (Layer 2 - Progress Truth)
Canonical in-flight execution state via reducer:

```rust
enum ExecutionState {
    Idle,               // No active execution
    Planning,           // Creating execution plan
    WaitingForApproval, // Paused for operator decision
    Executing,          // Running tools/mutations
    Validating,         // Running validation stages
    Responding,         // Formatting response
    Repairing,          // Auto-fixing validation failures
    Done,               // Completed successfully
    Failed,             // Terminal failure
    Blocked,            // Terminal blocked
}
```

**Rules**:
- ALL state changes flow through `reduce_execution_state()`
- Terminal states (Done, Failed, Blocked) are sticky
- State transitions are validated (no arbitrary jumps)
- UI renders from this state, never invents it

#### AuditLog (Layer 3 - Historical Truth)
Immutable append-only execution timeline:

```rust
struct AuditLog {
    events: Vec<AuditEvent>,  // Chronological order
}

enum AuditEventType {
    StateTransitionApplied,     // State changed
    StateTransitionRejected,    // Transition blocked
    StateTransitionNormalized,  // State corrected
    OutcomeFinalized,          // Terminal outcome set
    StepStarted,               // Step execution began
    StepCompleted,             // Step execution ended
    ApprovalRequested,         // Operator approval needed
    ApprovalResolved,          // Operator decision made
    RepairTriggered,           // Auto-repair started
    RepairCompleted,           // Auto-repair ended
    ValidationStarted,         // Validation began
    ValidationCompleted,       // Validation ended
}
```

**Rules**:
- Append-only: events are never modified or deleted
- Immutable: past events cannot change
- Complete: every transition is recorded
- Ordered: strict chronological sequence
- Sourced: all events from runtime, never UI

#### ReplayResult (Layer 4 - Reconstructable Truth)
Deterministic reconstruction from audit:

```rust
struct ReplayResult {
    initial_state: ExecutionState,
    final_state: ExecutionState,
    final_outcome: Option<ExecutionOutcome>,
    applied_transitions: Vec<ReplayedTransition>,
    rejected_transitions: Vec<ReplayedTransition>,
    normalized_transitions: Vec<ReplayedTransition>,
    warnings: Vec<ReplayWarning>,
    is_complete: bool,
}
```

**Rules**:
- Deterministic: same audit log → same result
- No runtime dependency: uses only audit events
- Validates: detects inconsistencies in audit log
- Read-only: never mutates state or executes code

#### ExecutionCheckpoint (Layer 5 - Resumable Truth)
Validated snapshot for crash recovery:

```rust
struct ExecutionCheckpoint {
    checkpoint_id: String,
    chain_id: String,
    active_step: Option<usize>,
    lifecycle_status: ChainLifecycleStatus,
    aggregated_outcome: Option<ExecutionOutcome>,
    execution_state: ExecutionState,
    audit_cursor: usize,           // Position in audit log
    workspace_hash: String,        // Blake3 of tracked files
    tracked_files: Vec<String>,
    validation_status: CheckpointValidationStatus,
    source: CheckpointSource,
    schema_version: u32,
}
```

**Rules**:
- Created only at safe boundaries (validated step completion, halt, pause)
- Never mid-mutation
- Includes workspace hash for integrity verification
- Includes audit cursor for consistency validation
- Resume validates against current workspace and replayed state

### Core Entities

#### SessionId
- Format: `forge-<timestamp>-<counter>`
- Generated per Forge worker run
- Not the same as Rasputin conversation ID

#### AgentState
Forge worker's execution state:
- `session_id`: Unique identity
- `iteration`: Current loop counter (0 to max_iterations)
- `files_read`: HashMap of FileRecord (read-before-write enforcement)
- `files_written`: Paths committed to disk
- `change_history`: Immutable change log
- `state_hash` / `previous_hash`: Integrity verification

#### ToolName
- Newtype with validation
- Rules: alphanumeric + underscore, starts with letter
- Canonical names: `read_file`, `write_file`, `apply_patch`, `list_dir`, `grep_search`

#### ExecutionMode (Forge Worker)
| Mode | Tools Allowed | Use Case |
|------|---------------|----------|
| `Analysis` | Read-only tools | Information gathering |
| `Edit` | All tools | File modifications |
| `Fix` | All tools | Error correction |
| `Batch` | All tools | Multi-file operations |

**Current runtime always uses `Edit` mode.**

#### ExperienceMode (TUI Product Layer)
Determines UI complexity and debug surface visibility:

| Mode | Audience | Inspector | Status Display | Composer |
|------|----------|-----------|----------------|----------|
| `Normal` | Daily users | Manual toggle | Human-readable | Conversational hints |
| `Operator` | Debug/audit | Auto-show | Technical details | Full mode toggle |

**Invariant**: Both modes render from identical truth layers; only presentation differs.

## Key Abstractions

### Bounded Execution

Forge enforces hard limits to prevent runaway agents:

| Limit | Default | Purpose |
|-------|---------|---------|
| `max_iterations` | 10 | Prevent infinite loops |
| `planner_timeout_seconds` | 30 | Cap planning time |
| Repair retries | 3 | Limit retry spam |
| Temperature | 0.0-0.1 | Reduce variance |
| Seed | 42 | Determinism anchor |

### Validation Gate

Mutation validation stages (in order):
1. **Syntax** — Parseable code check
2. **Build** — Compilation/type check
3. **Test** — Test suite execution

Outcomes: `Accept`, `Reject { reason, failed_stage }`, `Escalate { reason }`

### Read-Before-Write

Planner must read a file before writing or patching it:

```
write_file or apply_patch
         │
         ▼
ReadBeforeWriteGate::check()
         │
         ├─► File in files_read? ──► PASS
         │
         ├─► New file (not exist)? ──► PASS
         │
         └─► File not read, exists? ──► FAIL
```

### Risk Forecasting (V1.5)

Before chain execution, the system forecasts risks:

| Risk Type | Description | Level |
|-----------|-------------|-------|
| `GitConflict` | Uncommitted changes may conflict | Critical |
| `ValidationFailure` | Step may fail validation | Warning |
| `MissingContext` | Context files not provided | Caution |
| `ApprovalRequired` | Step requires approval | Warning |
| `UnprotectedWrite` | File modification without backup | Warning |

**Critical risks block execution** unless `--force` flag is provided.

### Interrupt Context (V1.5)

When execution is interrupted, context is captured:

- Current step number and description
- Total steps in chain
- Reason for interruption
- Recovery actions available

### State Integrity

AgentState maintains cryptographic hashes:
- `state_hash`: Current hash of all material fields
- `previous_hash`: Hash from previous iteration

Verification: `if state_hash != computed_hash { return Err(StateCorruption); }`

### Path Boundary Enforcement
All file system operations are validated against the repository root:

```rust
fn validate_path_boundary(path: &Path, working_dir: &Path) -> Result<PathBuf, ForgeError>
```

**Checks**:
- Path traversal (`../..`) rejected
- Symlink escapes blocked via canonicalization
- Operations outside repo boundary return `ForgeError::InvalidArgument`

**Purpose**: Prevent malicious or accidental file access outside the workspace.

### Command Execution Safety
Shell commands are gated through multi-layer safety:

| Layer | Mechanism |
|-------|-----------|
| **Allowlist** | Only safe commands permitted (cargo, npm, python, git, make) |
| **Destructive Detection** | rm, del, etc. require explicit confirmation |
| **Git Safety** | push, reset, clean, etc. require confirmation |
| **Timeouts** | All commands have execution limits |
| **Output Limits** | Prevents memory exhaustion |

### Local-Only Ollama Constraint
HTTP client is architecturally restricted to loopback:

```rust
assert!(
    endpoint.starts_with("http://127.0.0.1:")
        || endpoint.starts_with("http://[::1]:")
        || endpoint.starts_with("http://localhost:"),
    "Ollama endpoint must be loopback-only"
);
```

**Implication**: System cannot call cloud LLM APIs even if misconfigured.

## Terminology Glossary

| Term | Definition |
|------|------------|
| **Forge** | Bounded execution engine for code modification tasks |
| **Rasputin** | Terminal UI product that hosts chat and launches Forge |
| **Planner** | LLM component that decides tool calls vs completion |
| **Tool** | Registered capability (read_file, write_file, etc.) |
| **Mutation** | File system change (Write, Patch, Delete) |
| **Task** | User request processed as one Forge execution |
| **Iteration** | One planner → tool → validation cycle |
| **Inspector** | Right panel showing runtime, validation, logs, diff |
| **TUI** | Terminal User Interface (ratatui-based) |
| **Chain** | Multi-step objective with persistent state |
| **Commit** | Persisting mutations to disk after validation passes |
| **Revert** | Restoring snapshots after validation fails |
| **Worker** | Forge process spawned per task |
| **ExperienceMode** | Normal vs Operator UI complexity setting |
| **Path Boundary** | Repository-root validation for file operations |
| **Loopback-Only** | Ollama HTTP client restricted to 127.0.0.1/localhost |

## Design Patterns

### Fail-Closed
- Unknown tools return `ForgeError::UnknownTool`
- Missing arguments return `ForgeError::MissingArgument`
- Validation rejections block persistence
- State hash mismatches halt execution

### Separation of Concerns
```
Rasputin TUI ──► Product state (chat, repos)
      │
      ▼ spawns
Forge Worker ──► Execution state (tools, validation)
      │
      ▼ calls
   Ollama ──► LLM inference
```

### Event-Driven Observation
- Forge emits JSONL events during execution
- TUI parses and renders as inspector updates
- Events are observability, not control

### Deterministic Constraints
- Temperature clamped low
- Fixed random seed
- Bounded iterations
- Single output format (JSON)

## Data Flow Patterns

### Chat Flow
```
User message → Transcript → Ollama API → Assistant reply → Transcript
```

### Task Flow
```
task-like text or /goal → Qwen-Coder plan → PersistentChain → Spawn worker → JSONL events → Inspector updates → Final notice
```

### Validation Flow
```
Tool returns mutations → ValidationEngine → Stage checks → Accept/Reject → Commit or Revert
```

### Chain Flow (V1.5)
```
/goal → Generate steps → /preview → Risk check → /goal confirm or /chain resume → Execute step → Auto-resume? → Next step
```
