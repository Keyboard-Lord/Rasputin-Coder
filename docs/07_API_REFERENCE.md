# Rasputin API Reference

## Slash Commands (User Interface)

| Command | Arguments | Purpose |
|---------|-----------|---------|
| `/open <path>` | Repository path | Attach workspace |
| `/switch <path\|name>` | Path or recent repo name | Switch to different repo |
| `/project delete <path>` | Project path | Delete project folder |
| `/model` | None | Show active model |
| `/models` | None | Show installed Ollama models |
| `/model <tag>` | Model tag (e.g., `14b`) | Set planner model |
| `/config set planner_model <tag>` | Model tag | Same as `/model` |
| `/status` | None | Show runtime status |
| `/validate` | None | Run TUI validation pipeline |
| `/goal <description>` | Goal statement | Plan with Qwen-Coder and stage a bounded chain |
| `/goal confirm` | None | Accept staged goal plan and start bounded execution |
| Task-like plain text | Natural-language work request | Auto-route through goal planning and queued confirmation |
| `/task <description>` | Task description | Legacy/manual Forge task entrypoint |
| `/read <path>` | File path | Read file in project |
| `/write <path> -- <content>` | Path, content | Write file |
| `/replace <path> --find <text> --replace <text> [--hash <hash>]` | Path, find, replace | Replace text in file |
| `/run <command>` | Shell command | Execute shell command |
| `/approve` | None | Approve pending action |
| `/deny` | None | Deny pending action |
| `/help` | None | Show help |
| `/quit` | None | Exit Rasputin |
| `/rlef status` | None | Show RLEF statistics |
| `/rlef clear` | None | Clear learned hints |
| `/rlef disable <class> -- <guidance>` | Class, guidance | Disable hint |

### Chain Management Commands (Phase B)

| Command | Arguments | Purpose |
|---------|-----------|---------|
| `/chains` | None | List all non-archived chains |
| `/chain status [id]` | Optional chain ID | Show chain details (active if no ID) |
| `/chain switch <id>` | Chain ID | Set active chain, bind to conversation |
| `/chain archive <id>` | Chain ID | Archive chain (fail-closed if running) |
| `/chain resume <id>` | Chain ID | Resume chain execution with policy check |
| `/resume` | None | Resume active chain (alias) |
| `/continue` | None | Resume active chain (alias) |
| `/plan` | None | Show plan for active chain |
| `/preview` | None | Preview chain with risk forecast |
| `/stop` | None | Interrupt current execution |
| `/checkpoint` | None | Show checkpoint status |
| `/checkpoint status` | None | Detailed checkpoint status |
| `/plan context` | None | Show context files |
| `/plan checkpoints` | None | Show checkpoint plan |

## Keyboard Shortcuts

| Key | Context | Action |
|-----|---------|--------|
| `Enter` | Composer | Submit input |
| `Esc` | Any | Leave editing mode |
| `i` | Navigation | Enter editing mode |
| `Tab` | Editing | Toggle inspector |
| `Tab` / `Shift+Tab` | Navigation | Cycle focus |
| `Enter` | Navigation | Activate focused control |
| Mouse click | Any | Activate buttons, tabs, entries |

## Public Types (Rust)

### SessionId
```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SessionId(String);

impl SessionId {
    pub fn new() -> Self;  // forge-<timestamp>-<counter>
    pub fn from_string(s: String) -> Self;
}
```

### ToolName
```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ToolName(String);

impl ToolName {
    pub fn new(name: &str) -> Result<Self, ForgeError>;
    pub fn as_str(&self) -> &str;
}
```

### ExecutionMode
```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    Analysis,  // Read-only tools only
    Edit,      // All tools allowed (default)
    Fix,       // Error correction mode
    Batch,     // Multi-file automated mode
}
```

### ToolCall
```rust
#[derive(Debug, Clone)]
pub struct ToolCall {
    pub name: ToolName,
    pub arguments: ToolArguments,
}
```

### ToolArguments
```rust
#[derive(Debug, Clone, Default)]
pub struct ToolArguments {
    pub fn new() -> Self;
    pub fn set(&mut self, key: &str, value: &str);
    pub fn get(&self, key: &str) -> Option<&str>;
    pub fn require(&self, key: &str) -> Result<&str, ForgeError>;
}
```

### ToolResult
```rust
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub success: bool,
    pub output: Option<String>,
    pub error: Option<ToolError>,
    pub mutations: Vec<Mutation>,
    pub execution_time_ms: u64,
}
```

### AgentState (Forge Worker State)
```rust
pub struct AgentState {
    pub session_id: SessionId,
    pub iteration: u32,
    pub max_iterations: u32,
    pub status: SessionStatus,
    pub files_written: Vec<PathBuf>,
    pub files_read: HashMap<PathBuf, FileRecord>,
    pub change_history: Vec<ChangeRecord>,
    pub completion_reason: Option<CompletionReason>,
    pub state_hash: String,
    pub previous_hash: String,
    pub snapshots: HashMap<PathBuf, FileSnapshot>,
}
```

**Methods**:
```rust
impl AgentState {
    pub fn to_json(&self) -> String;
    pub fn from_json(json: &str) -> Result<Self, ValidationError>;
    pub fn save_default(&self) -> Result<(), std::io::Error>;
    pub fn load_default() -> Result<Self, ValidationError>;
}
```

## Chain Types (Phase C)

### PersistentChain

```rust
pub struct PersistentChain {
    pub id: String,
    pub name: String,
    pub objective: String,
    pub status: ChainLifecycleStatus,
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
```

### ChainLifecycleStatus

```rust
pub enum ChainLifecycleStatus {
    Draft,
    Ready,
    Running,
    WaitingForApproval,
    Halted,
    Failed,
    Complete,
    Archived,
}
```

### PersistentChainStep

```rust
pub struct PersistentChainStep {
    pub id: String,
    pub description: String,
    pub status: ChainStepStatus,
    pub tool_calls: Vec<String>,
    pub result_summary: Option<String>,
    pub validation_passed: Option<bool>,
    pub started_at: Option<DateTime<Local>>,
    pub completed_at: Option<DateTime<Local>>,
    pub error_message: Option<String>,
}
```

### ChainStepStatus

```rust
pub enum ChainStepStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Blocked,
}
```

### ChainPolicy

```rust
pub struct ChainPolicy {
    pub max_steps: u32,
    pub require_validation_each_step: bool,
    pub halt_on_failure: bool,
    pub max_consecutive_failures: u32,
    pub auto_retry_on_validation_failure: bool,
    pub require_approval_after_step_count: Option<u32>,
}
```

**Default Policy**:
```rust
ChainPolicy {
    max_steps: 100,
    require_validation_each_step: true,
    halt_on_failure: true,
    max_consecutive_failures: 3,
    auto_retry_on_validation_failure: false,
    require_approval_after_step_count: None,
}
```

## Tool Registry API

### Tool Trait
```rust
pub trait Tool: Send + Sync {
    fn name(&self) -> ToolName;
    fn description(&self) -> &str;
    fn allowed_in_mode(&self, mode: ExecutionMode) -> bool;
    fn execute(&self, args: &ToolArguments, ctx: &ExecutionContext) -> Result<ToolResult, ForgeError>;
}
```

### ToolRegistry
```rust
impl ToolRegistry {
    pub fn new() -> Self;  // Register all built-in tools
    pub fn resolve(&self, name: &ToolName) -> Result<&dyn Tool, ForgeError>;
    pub fn has_tool(&self, name: &ToolName) -> bool;
    pub fn list_tools(&self) -> Vec<ToolName>;
}
```

### ToolExecutor
```rust
impl ToolExecutor {
    pub fn new() -> Self;
    pub fn execute(&self, call: &ToolCall, ctx: &ExecutionContext) -> Result<ToolResult, ForgeError>;
}
```

## Tool Schemas

### read_file
**Arguments**:
- `path` (required): File path
- `offset` (optional): Line offset for partial read
- `limit` (optional): Max lines to read

**Returns**: File content and metadata

### write_file
**Arguments**:
- `path` (required): Target file path
- `content` (required): File content

**Returns**: `Write` mutation record

### apply_patch
**Arguments**:
- `file_path` (required): File to patch
- `old_text` (required): Text to replace
- `new_text` (required): Replacement text
- `expected_hash` (required): Content hash for verification

**Returns**: `Patch` mutation record

### grep_search
**Arguments**:
- `query` (required): Search pattern
- `path` (optional): Directory to search
- `case_sensitive` (optional): Boolean
- `max_results` (optional): Limit results
- `file_pattern` (optional): Glob pattern
- `context_lines` (optional): Lines of context
- `use_regex` (optional): Boolean

**Returns**: List of matches with file path, line number, content, and context

**Bounds**: Results limited to prevent context explosion (default: 100 matches max)

### list_dir
**Arguments**:
- `path` (required): Directory path
- `recursive` (optional): Boolean
- `include_hidden` (optional): Boolean
- `file_type` (optional): "file" or "directory"
- `max_depth` (optional): Recursion limit
- `max_entries` (optional): Result limit

**Returns**: List of entries with path, name, type, and size

**Bounds**: Max depth 5, max entries 1000. Path traversal blocked outside repo.

### execute_command
**Arguments**:
- `command` (required): Shell command
- `working_dir` (optional): Working directory
- `timeout_seconds` (optional): Execution timeout
- `capture_stderr` (optional): Boolean
- `max_output_lines` (optional): Truncate output
- `require_confirmation` (optional): Safety flag

**Note**: Not exposed to planner in current runtime policy.

### browser_preview
**Arguments**:
- `directory` (optional): Serve directory
- `port` (optional): Server port
- `open` (optional): Auto-open browser

**Note**: Not exposed to planner in current runtime policy.

## Runtime Configuration

### RuntimeConfig (Forge Worker)
```rust
pub struct RuntimeConfig {
    pub max_iterations: u32,           // Default: 10
    pub task: String,                // Task description
    pub auto_revert: bool,           // Default: true
    pub mode: ExecutionMode,         // Default: Edit
    pub planner_type: String,        // Default: "http"
    pub planner_endpoint: String,    // Default: http://127.0.0.1:11434
    pub planner_model: String,       // Default: qwen2.5-coder:14b
    pub planner_timeout_seconds: u32, // Default: 30
    pub planner_temperature: f32,    // Default: 0.0
    pub planner_seed: u64,           // Default: 42
    pub css_compression: bool,       // Auto-enabled for 14B+ models
}
```

## Environment Variables

| Variable | Purpose | Default |
|----------|---------|---------|
| `FORGE_PLANNER_MODEL` | Override model | (workspace config) |
| `FORGE_PLANNER_ENDPOINT` | Ollama URL | http://127.0.0.1:11434 |
| `FORGE_PLANNER_TEMPERATURE` | Planner temperature | 0.0 |
| `FORGE_PLANNER_SEED` | Random seed | 42 |
| `FORGE_CSS_COMPRESSION` | Enable compression | false (auto for 14B+) |
| `FORGE_OUTPUT_MODE` | Output format | human (or jsonl) |
| `FORGE_RUNTIME_BIN` | Worker binary path | auto-detected |
| `OLLAMA_HOST` | Ollama base URL | http://localhost:11434 |

## Additional Types (V1.5)

### Risk Types

```rust
pub enum RiskType {
    GitConflict,
    ValidationFailure,
    MissingContext,
    ApprovalRequired,
    UnprotectedWrite,
}

pub enum RiskLevel {
    Safe,
    Caution,
    Warning,
    Critical,  // Blocks execution
}

pub struct Risk {
    pub risk_type: RiskType,
    pub level: RiskLevel,
    pub description: String,
    pub step_index: Option<usize>,
}
```

### Task Intake Types

```rust
pub struct StructuredTaskIntake {
    pub original_request: String,
    pub interpreted_objective: String,
    pub task_class: TaskClass,
    pub risk_level: TaskRiskLevel,
    pub likely_targets: Vec<String>,
    pub requires_clarification: bool,
}

pub enum TaskClass {
    ReadOnlyAnalysis,
    SingleFileEdit,
    MultiFileEdit,
    Refactor,
    ValidationOnly,
    DebugFix,
    Unknown,
}
```

### Git Grounding Types

```rust
pub struct GitGrounding {
    pub repo_detected: bool,
    pub branch_name: Option<String>,
    pub head_commit: Option<String>,
    pub is_dirty: bool,
    pub modified_files: Vec<GitFileStatus>,
    pub staged_files: Vec<GitFileStatus>,
    pub untracked_files: Vec<GitFileStatus>,
}

pub struct TaskStartCheckResult {
    pub allowed: bool,
    pub warnings: Vec<TaskStartWarning>,
    pub requires_approval: bool,
}
```

### Approval Checkpoint Types

```rust
pub struct ApprovalCheckpoint {
    pub step_index: u32,
    pub reason: String,
    pub checkpoint_type: ApprovalCheckpointType,
    pub state: ApprovalCheckpointState,
}

pub enum ApprovalCheckpointType {
    PreExecution,
    PreMutationCommit,
    PostValidationPreAdvance,
    ReplayMismatchReview,
}

pub enum ApprovalCheckpointState {
    Pending,
    Approved,
    Denied,
    Expired,
}
```

## JSONL Event Format (Worker → TUI)

Forge emits structured events during execution:

```json
{"type":"init","session_id":"forge-1234567890-00000001","timestamp":1234567890}
{"type":"iteration_start","iteration":0}
{"type":"planner_output","output_type":"tool_call","tool":"write_file"}
{"type":"tool_execute","tool":"write_file","path":"src/main.rs"}
{"type":"validation_start","stage":"syntax"}
{"type":"validation_stage","stage":"syntax","status":"passed"}
{"type":"state_committed","files_written":["src/main.rs"]}
{"type":"completion","reason":"Task completed successfully"}
{"type":"finished","success":true,"iterations":2}
```

Event types: `init`, `iteration_start`, `planner_output`, `protocol_validation`, `tool_execute`, `tool_result`, `mutations_detected`, `validation_start`, `validation_stage`, `validation_result`, `state_committed`, `completion`, `failure`, `repair_loop`, `finished`, `task_intake`, `git_grounding`, `checkpoint_created`, `checkpoint_resolved`

## Error Types

```rust
pub enum ForgeError {
    InvalidToolName(String),
    UnknownTool(ToolName),
    MissingArgument(String),
    InvalidArgument(String),
    ToolNotAllowed { tool: ToolName, mode: ExecutionMode },
    IoError(String),
    SerializationError(String),
    StateCorruption { expected: String, actual: String },
    ValidationError(String),
    PlannerError(String),
    RuntimeError(String),
}
```
