# Rasputin Data Model

## Overview

Rasputin maintains two distinct state systems:
1. **Rasputin TUI State** — Product state (persistent)
2. **Forge Worker State** — Per-task execution state (in-memory, per process)

## Rasputin TUI Persistence

### Storage Location
```
~/.local/share/rasputin/state.json
```

### PersistentState Schema

```rust
struct PersistentState {
    version: String,                    // "1.0"
    last_updated: DateTime<Local>,
    active_repo: Option<String>,        // Path to active workspace
    active_conversation: Option<String>, // Conversation ID
    active_chain_id: Option<String>,     // Active chain ID (Phase B)
    recent_repos: Vec<RecentRepo>,
    conversations: Vec<PersistentConversation>,
    chains: Vec<PersistentChain>,        // Chain registry (Phase B)
    last_model_status: Option<ModelStatus>,
    chain_policy: ChainPolicy,           // Execution constraints (Phase B)
}
```

### RecentRepo

```rust
struct RecentRepo {
    path: String,
    name: String,
    last_opened: DateTime<Local>,
    ollama_model: Option<String>,
}
```

### PersistentConversation

```rust
struct PersistentConversation {
    id: String,
    title: String,
    repo_path: Option<String>,
    project_id: Option<String>,
    mode: String,                       // "chat", "task", etc.
    execution: PersistentExecutionState,
    inspector: PersistentInspectorState,
    messages: Vec<PersistentMessage>,
    runtime_events: Vec<PersistentEvent>,
    created_at: DateTime<Local>,
    updated_at: DateTime<Local>,
    archived: bool,
    chain_id: Option<String>,           // Conversation ↔ Chain binding (Phase B)
}
```

### PersistentMessage

```rust
struct PersistentMessage {
    id: String,
    role: String,                       // "user", "assistant", "system"
    content: String,
    timestamp: DateTime<Local>,
}
```

### PersistentExecutionState

```rust
struct PersistentExecutionState {
    mode: String,                       // "plan", "execute"
    state: String,                      // "idle", "running", "complete", "error"
    active_objective: Option<String>,
    last_action: String,
    current_step: Option<String>,
    step_index: Option<u32>,
    step_total: Option<u32>,
    active_tool: Option<String>,
    planner_output: Vec<String>,
    tool_calls: Vec<String>,
    file_writes: Vec<String>,
    validation_summary: Option<String>,
    block_reason: Option<String>,
    block_fix: Option<String>,
    block_command: Option<String>,
}
```

### PersistentInspectorState

```rust
struct PersistentInspectorState {
    show_inspector: bool,
    active_tab: String,                 // "Runtime", "Validation", "Logs", "Preview", "Diff"
    runtime_scroll: usize,
    validation_scroll: usize,
    logs_scroll: usize,
    preview_scroll: usize,
    diff_scroll: usize,
}
```

### PersistentEvent

```rust
struct PersistentEvent {
    timestamp: DateTime<Local>,
    source: String,                     // "runtime", "validation", "planner"
    level: String,                      // "info", "warn", "error"
    message: String,
}
```

### ModelStatus

```rust
struct ModelStatus {
    configured: Option<String>,         // Model from config
    active: Option<String>,             // Actually resolved model
    ollama_connected: bool,
    verified_at: DateTime<Local>,
}
```

## Chain State (Phase B)

### PersistentChain

```rust
struct PersistentChain {
    id: String,                         // chain-<timestamp>-<random>
    name: String,                       // Human-readable name
    objective: String,                  // High-level goal
    status: ChainLifecycleStatus,
    steps: Vec<PersistentChainStep>,
    active_step: Option<usize>,         // Current step index
    repo_path: Option<String>,          // Associated workspace
    conversation_id: Option<String>,    // Associated conversation
    created_at: DateTime<Local>,
    updated_at: DateTime<Local>,
    completed_at: Option<DateTime<Local>>,
    archived: bool,
    total_steps_executed: u32,
    total_steps_failed: u32,
}
```

### ChainLifecycleStatus

```rust
enum ChainLifecycleStatus {
    Draft,              // Initial state, steps being defined
    Ready,              // Steps defined, ready to run
    Running,            // Currently executing
    WaitingForApproval, // Paused for operator approval
    Halted,             // Stopped by operator or policy
    Failed,             // Terminal: step failure
    Complete,           // Terminal: all steps done
    Archived,           // Terminal: manually archived
}
```

### PersistentChainStep

```rust
struct PersistentChainStep {
    id: String,
    description: String,              // What this step does
    status: ChainStepStatus,
    tool_calls: Vec<String>,           // Tools invoked
    result_summary: Option<String>,    // Execution outcome
    validation_passed: Option<bool>,    // Validation gate result
    started_at: Option<DateTime<Local>>,
    completed_at: Option<DateTime<Local>>,
    error_message: Option<String>,
}
```

### ChainStepStatus

```rust
enum ChainStepStatus {
    Pending,    // Not yet started
    Running,    // Currently executing
    Completed,  // Successfully finished
    Failed,     // Execution or validation failed
    Blocked,    // Waiting for dependency/unblock
}
```

### ChainPolicy (Phase C)

```rust
struct ChainPolicy {
    max_steps: u32,                           // Default: 100
    require_validation_each_step: bool,       // Default: true
    halt_on_failure: bool,                  // Default: true
    max_consecutive_failures: u32,         // Default: 3
    auto_retry_on_validation_failure: bool, // Default: false
    require_approval_after_step_count: Option<u32>, // Default: None
    auto_resume: bool,                      // Default: false (Phase C: opt-in autonomous continuation)
}
```

**V1.5 Auto-Resume**: When `auto_resume: true`, chains automatically continue execution after successful step completion, bounded by `max_steps` and `halt_on_failure`.

## V1.5 Data Types

### InterruptContext

```rust
struct InterruptContext {
    chain_id: String,
    step_index: usize,
    total_steps: usize,
    reason: InterruptReason,
    recovery_actions: Vec<String>,
}

enum InterruptReason {
    UserStopped,
    SignalReceived,
    PolicyViolation,
}
```

### ExecutionPreview

```rust
struct ExecutionPreview {
    chain_id: String,
    upcoming_steps: Vec<StepPreview>,
    detected_risks: Vec<Risk>,
    overall_risk_level: RiskLevel,
    policy_check: PolicyCheckResult,
}

struct StepPreview {
    step_index: usize,
    description: String,
    status: ChainStepStatus,
    estimated_risk: RiskLevel,
}
```

### Risk

```rust
struct Risk {
    risk_type: RiskType,
    level: RiskLevel,
    description: String,
    step_index: Option<usize>,
}

enum RiskType {
    GitConflict,
    ValidationFailure,
    MissingContext,
    ApprovalRequired,
    UnprotectedWrite,
}

enum RiskLevel {
    Safe,
    Caution,
    Warning,
    Critical,  // Blocks execution
}
```

### GitGrounding

```rust
struct GitGrounding {
    repo_detected: bool,
    branch_name: Option<String>,
    head_commit: Option<String>,
    is_dirty: bool,
    modified_files: Vec<GitFileStatus>,    // max 100
    staged_files: Vec<GitFileStatus>,      // max 100
    untracked_files: Vec<GitFileStatus>,   // max 100
    recent_commits: Vec<GitCommitSummary>, // max 10
}

struct GitFileStatus {
    path: String,
    status: String,  // M, A, D, ??, etc.
}

struct GitCommitSummary {
    short_hash: String,
    subject: String,
    author: Option<String>,
}
```

### StructuredTaskIntake

```rust
struct StructuredTaskIntake {
    original_request: String,
    interpreted_objective: String,
    task_class: TaskClass,
    risk_level: TaskRiskLevel,
    likely_targets: Vec<String>,
    requires_clarification: bool,
    clarification_questions: Vec<String>,
}

enum TaskClass {
    ReadOnlyAnalysis,
    SingleFileEdit,
    MultiFileEdit,
    Refactor,
    ValidationOnly,
    DebugFix,
    Unknown,
}
```

### ApprovalCheckpoint

```rust
struct ApprovalCheckpoint {
    step_index: u32,
    reason: String,
    checkpoint_type: ApprovalCheckpointType,
    state: ApprovalCheckpointState,
    created_at: u64,
    resolved_at: Option<u64>,
    resolved_by: Option<String>,
}

enum ApprovalCheckpointType {
    PreExecution,
    PreMutationCommit,
    PostValidationPreAdvance,
    ReplayMismatchReview,
}

enum ApprovalCheckpointState {
    Pending,
    Approved,
    Denied,
    Expired,
}
```

## Forge Worker State (AgentState)

### Storage Location (API exists but not auto-wired)
```
~/.local/share/forge/session.json
```

### AgentState Schema

```rust
struct AgentState {
    session_id: SessionId,              // forge-<timestamp>-<counter>
    iteration: u32,                     // Current iteration (0 to max)
    max_iterations: u32,              // Default: 10
    status: SessionStatus,              // Initializing, Running, Complete, Error, Halted
    files_written: Vec<PathBuf>,        // Committed files
    files_read: HashMap<PathBuf, FileRecord>,
    change_history: Vec<ChangeRecord>,
    completion_reason: Option<CompletionReason>,
    state_hash: String,                 // Integrity verification
    previous_hash: String,              // Previous iteration hash
    snapshots: HashMap<PathBuf, FileSnapshot>, // For revert
}
```

### SessionStatus

```rust
enum SessionStatus {
    Initializing,
    Running,
    Complete,
    Error,
    Halted,
}
```

### FileRecord

```rust
struct FileRecord {
    path: PathBuf,
    content_hash: String,               // SHA256
    size: u64,
    total_lines: usize,
    is_full_read: bool,                 // Full vs partial read
    iteration: u32,                     // When read occurred
    excerpt: String,                    // First 200 chars preview
}
```

### ChangeRecord

```rust
struct ChangeRecord {
    iteration: u32,
    timestamp: u64,
    path: PathBuf,
    change_type: ChangeType,
    tool_used: String,
    description: String,
}
```

### ChangeType

```rust
enum ChangeType {
    Create,
    Modify,
    Delete,
}
```

### FileSnapshot

```rust
struct FileSnapshot {
    path: PathBuf,
    original_content: Option<Vec<u8>>,  // None = file didn't exist
    captured_at: u64,
}
```

### Mutation Types

```rust
struct Mutation {
    path: PathBuf,
    mutation_type: MutationType,
    content_hash_before: Option<String>,
    content_hash_after: Option<String>,
}

enum MutationType {
    Write,
    Patch,
    Delete,
    Move,
}
```

## Tool Call Data

### ToolCall

```rust
struct ToolCall {
    name: ToolName,
    arguments: ToolArguments,
}
```

### ToolArguments

```rust
struct ToolArguments {
    inner: HashMap<String, String>,
}
```

### ToolResult

```rust
struct ToolResult {
    success: bool,
    output: Option<String>,
    error: Option<ToolError>,
    mutations: Vec<Mutation>,
    execution_time_ms: u64,
}
```

## Validation Data

### ValidationReport

```rust
struct ValidationReport {
    outcome: ValidationOutcome,
    stages: Vec<StageResult>,
    failed_stage: Option<String>,
    requires_revert: bool,
}
```

### ValidationOutcome

```rust
enum ValidationOutcome {
    Accept,
    Reject { reason: String, failed_stage: String },
    Escalate { reason: String },
}
```

### StageResult

```rust
struct StageResult {
    name: String,                       // "syntax", "lint", "build", "test"
    status: StageStatus,
    message: Option<String>,
}

enum StageStatus {
    Passed,
    Failed(String),
    Skipped(String),
}
```

## Workspace Configuration

### Resolution Order

1. `.forge/config.yaml`
2. `.forge/config.yml`
3. `rasputin.json`

### Config Schema (YAML)

```yaml
planner:
  model: qwen2.5-coder:14b

ollama:
  model: qwen2.5-coder:14b
```

### Config Schema (JSON - rasputin.json)

```json
{
  "ollama_model": "qwen2.5-coder:14b"
}
```

## V1.6 Audit, Replay, and Checkpoint Data Types

### ExecutionOutcome

Single authoritative terminal outcome per chain (Layer 1 Truth):

```rust
enum ExecutionOutcome {
    Success,              // All steps completed, no warnings
    SuccessWithWarnings,  // Completed with non-fatal warnings
    Failed,               // Step failed, halted
    Blocked,              // Blocked by approval denial or system issue
}
```

**Location**: `state.rs` and `persistence.rs`

### ExecutionState

Canonical in-flight execution progress (Layer 2 Truth):

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

**Reducer**: `reduce_execution_state()` in `state.rs`

### AuditLog

Immutable append-only execution timeline (Layer 3 Truth):

```rust
struct AuditLog {
    events: Vec<AuditEvent>,  // Chronological, append-only
}

struct AuditEvent {
    timestamp: DateTime<Utc>,
    event_type: AuditEventType,
    previous_state: Option<ExecutionState>,
    next_state: Option<ExecutionState>,
    triggering_event: Option<String>,
    step_id: Option<String>,
    chain_id: Option<String>,
    task: Option<String>,
    reason: Option<String>,
    metadata: Option<String>,
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

**Location**: `state.rs`

**Rules**:
- Append-only: events never modified or deleted
- Immutable: past events cannot change
- Complete: every transition recorded
- Ordered: strict chronological sequence

### ReplayResult

Deterministic reconstruction from audit (Layer 4 Truth):

```rust
struct ReplayResult {
    initial_state: ExecutionState,
    final_state: ExecutionState,
    final_outcome: Option<ExecutionOutcome>,
    applied_transitions: Vec<ReplayedTransition>,
    rejected_transitions: Vec<ReplayedTransition>,
    normalized_transitions: Vec<ReplayedTransition>,
    outcome_event: Option<ReplayedOutcome>,
    warnings: Vec<ReplayWarning>,
    is_complete: bool,
    events_processed: usize,
}

struct ReplayedTransition {
    timestamp: DateTime<Utc>,
    transition_type: ReplayTransitionType,  // Applied, Rejected, Normalized
    from_state: ExecutionState,
    to_state: ExecutionState,
    trigger: Option<String>,
    reason: Option<String>,
    step_id: Option<String>,
    task: Option<String>,
    event_index: usize,
}

enum ReplayWarning {
    MissingOutcome,
    InconsistentTransition { event_index: usize, expected: ExecutionState, actual: ExecutionState },
    MultipleOutcomes { first_index: usize, second_index: usize },
    ImpossibleSequence { event_index: usize, description: String },
    GapDetected { after_event_index: usize },
}
```

**Function**: `replay_audit_log()` in `state.rs`

**Rules**:
- Deterministic: same audit log → same result
- No runtime dependency: uses only audit events
- Read-only: never mutates state

### ExecutionCheckpoint

Validated snapshot for crash recovery (Layer 5 Truth):

```rust
struct ExecutionCheckpoint {
    checkpoint_id: String,
    chain_id: String,
    active_step: Option<usize>,      // Next step to execute
    lifecycle_status: ChainLifecycleStatus,
    aggregated_outcome: Option<ExecutionOutcome>,
    execution_state: ExecutionState,   // State at checkpoint boundary
    audit_cursor: usize,               // Last processed audit event index
    workspace_hash: String,            // Blake3 hash of tracked files
    tracked_files: Vec<String>,
    validation_status: CheckpointValidationStatus,
    source: CheckpointSource,
    message: Option<String>,
    created_at: DateTime<Local>,
    schema_version: u32,
}

enum CheckpointSource {
    Manual,              // Operator-initiated
    AutoValidatedStep,   // After successful step completion
    SafeHalt,            // At explicit safe halt
    ApprovalPause,       // At approval boundary
    CrashRecovery,       // Created during recovery
    ExplicitSave,        // At explicit save point
}

enum CheckpointValidationStatus {
    Valid,      // Validated and ready
    Warning,    // Has warnings but may be resumable
    Invalid,    // Cannot be used for resume
    Unchecked,  // Validation not yet performed
}
```

**Storage**: `~/.local/share/rasputin/chains/{chain_id}/checkpoints/{checkpoint_id}.json`

**Manager**: `CheckpointManager` in `persistence.rs`

### WorkspaceHash

Integrity verification for checkpoint resume:

```rust
struct WorkspaceHash {
    overall_hash: String,         // Combined Blake3 hash
    file_hashes: Vec<FileHash>,
    calculated_at: DateTime<Local>,
    base_path: String,
}

struct FileHash {
    path: String,
    hash: String,      // Blake3
    size: u64,
    modified: DateTime<Local>,
}
```

## Data Flow Diagram

```mermaid
flowchart TB
    subgraph Truth["System Truth Layers (V1.6)"]
        L1[ExecutionOutcome]
        L2[ExecutionState]
        L3[AuditLog]
        L4[ReplayResult]
        L5[ExecutionCheckpoint]
    end

    subgraph TUI["Rasputin TUI State"]
        PS[PersistentState]
        PC[PersistentConversation]
        PM[PersistentMessage]
        PE[PersistentEvent]
        AL[AuditLog]
    end

    subgraph Worker["Forge Worker State (per task)"]
        AS[AgentState]
        FR[FileRecord]
        CR[ChangeRecord]
        FS[FileSnapshot]
    end

    subgraph Disk["Disk Storage"]
        TUI_FILE[~/.local/share/rasputin/state.json]
        CHK_DIR[~/.local/share/rasputin/chains/{id}/checkpoints/]
        REPO_FILES[Repository files]
    end

    L1 --> L2 --> L3 --> L4 --> L5
    AL --> L3
    PS --> TUI_FILE
    L5 --> CHK_DIR
    AS -->|mutations| REPO_FILES
```

## Key Constraints

1. **Truth Layer Invariant**: Each layer derives from the layer below. UI NEVER invents truth.
2. **Audit Immutability**: Events are append-only; past events never change.
3. **Replay Determinism**: Same audit log always produces same reconstruction.
4. **Checkpoint Validation**: Resume requires workspace hash match AND replay state match.
5. **Fail-Closed**: Stale, corrupted, or divergent checkpoints block resume.
6. **No unified session**: TUI and Forge state are separate but TUI has audit/replay truth.
7. **Read-before-write enforced**: File must be in `files_read` before mutation
