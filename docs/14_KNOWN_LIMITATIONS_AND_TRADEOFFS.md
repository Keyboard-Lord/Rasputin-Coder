# Rasputin Known Limitations and Tradeoffs

## Executive Summary

This document describes the **current boundaries** of Rasputin's implementation. These are not permanent philosophical refusals—they are the edges of what exists today, documented so the path forward is clear.

Rasputin is evolving toward **bounded autonomy**: controlled, inspectable, validation-gated agent behavior that operates within hard safety constraints. The limitations listed here define the gap between current single-run execution and the target state of structured multi-step autonomy.

Every limitation includes: what exists now, why it exists, what impact it has, and what constraints must be preserved when addressing it.

## Current System Boundaries

### 1. ~~Single-Step Execution Only~~ RESOLVED

**Status**: Chain execution implemented as of current sprint.

**Previous State**: Each Forge task was an isolated, non-resumable execution with no structured mechanism for chaining multiple bounded steps.

**Current State**: 
- `ChainExecutor` provides structured multi-step task chains
- Each step runs through the full validation pipeline (format → lint → build → test)
- Steps are validated independently; failure halts the chain deterministically
- Chain state is inspectable via `ChainEvent` audit log

**Remaining Constraints**:
- Goal mode is wired to TUI chain execution; `/task` remains a manual Forge entrypoint
- Resume from checkpoint requires explicit approval (intentional safety feature)
- No mid-step approval checkpoints within a chain step

---

### 2. ~~No Chain-Aware Memory Between Validated Runs~~ RESOLVED (Phase B/C)

**Status**: TUI-side chain persistence with resume flow and autonomous continuation implemented.

**Previous State**: Execution state was ephemeral. `AgentState` existed only in-memory and was discarded when the Forge worker process exited.

**Current State (Phase B)**:
- `PersistentState` now contains `chains: Vec<PersistentChain>` that survives restart
- `PersistentState::create_chain()` creates durable chain records with lifecycle tracking
- `PersistentState::archive_chain()` provides fail-closed chain archival
- `/chain resume <id>` bridges from persisted chain to Forge execution with policy checks
- `ChainPolicy` enforces max_steps, halt_on_failure, and validation requirements
- Conversation-to-chain binding via `conversation.chain_id`

**Current State (Phase C)**:
- `handle_chain_step_completion()` updates step status, records result, updates stats
- `try_auto_resume_chain()` triggers automatic continuation if `auto_resume: true`
- Chains become self-propelling within policy bounds (max_steps, halt_on_failure)
- Operator can override via `/chain archive` or `/chain switch` at any time

**Remaining Constraints**:
- Corrupted or tampered checkpoints are rejected (fail-closed)
- Workspace hash mismatch prevents resume (files changed since checkpoint)

---

### 3. ~~Limited Planner Discovery Capabilities~~ RESOLVED

**Status**: Discovery tools now planner-visible as of [DATE].

**Previous State**: Planner only saw 3 tools: `read_file`, `write_file`, `apply_patch`. Discovery tools (`list_dir`, `grep_search`) were registered but not exposed.

**Current State**: Planner has access to discovery and intelligence tools:
- `read_file`, `write_file`, `apply_patch` (original mutation tools)
- `list_dir` (directory exploration with bounds)
- `grep_search` (pattern search with bounds)
- `dependency_graph` (module dependency analysis with depth limits)
- `symbol_index` (code symbol search with result caps)
- `entrypoint_detector` (finds main/entry functions with enumeration caps)
- `lint_runner`, `test_runner` (validation tools with bounded execution)

**Remaining Constraints**:
- No automatic discovery on task start (must be explicitly invoked)
- Boundedness limits enforced but not yet user-configurable
- No semantic understanding (symbol relationships not cross-referenced)

**Why It Exists**: Minimal tool surface reduces decision space and improves determinism. Discovery was deferred until bounded context assembly could be designed.

**Impact**:
- User must provide explicit file paths in task descriptions
- Planner cannot explore codebase structure to find relevant files
- No automatic dependency or import graph traversal
- Manual path provision creates friction for large refactors

**Constraints for Resolution**:
- Discovery must be bounded (max directories, max files, depth limits)
- Context assembly must be deterministic and reproducible
- Token budgeting must prevent context overflow
- Read-before-write gate must still enforce explicit file reads for mutation

**Target Direction**: Bounded, deterministic repo discovery with explicit context assembly and strict token budgets.

---

### 4. ~~No Structured Task Chaining~~ RESOLVED (Phase B/C)

**Status**: Full chain system with persistence, command surface, step tracking, and autonomous continuation implemented.

**Previous State**: There was no first-class concept of a "task chain" or "objective spanning multiple steps." Each `/task` was independent.

**Current State (Phase B)**:
- `PersistentChain` struct with full lifecycle (Draft→Ready→Running→Complete/Failed/Halted/Archived)
- `PersistentChainStep` with status tracking (Pending, Running, Completed, Failed, Blocked)
- Chain persistence across restarts via `PersistentState.chains`
- **Command surface exposed**: `/chains`, `/chain status`, `/chain switch`, `/chain archive`, `/chain resume`, `/resume`, `/continue`, `/plan`
- `ChainPolicy` enforces bounded execution (max_steps, halt_on_failure, require_validation_each_step)
- Chain-to-conversation binding for context continuity

**Current State (Phase C)**:
- `handle_chain_step_completion()` wired to `RuntimeEvent::Finished`
- Step result tracking: status, result_summary, validation_passed, completed_at
- Autonomous continuation: `auto_resume` triggers automatic `/chain resume`
- Chain step advancement automatic when policy allows
- Operator override: `/chain archive`, `/chain switch` halt auto-resume

**Remaining Constraints**:
- Task-like plain text and `/goal` create planned chains; `/task` remains a legacy/manual task path
- No mid-step approval checkpoints (only chain-level halt/resume)

---

### 5. No Intermediate Approval Checkpoints

**Current State**: Forge executes to completion without intermediate user review. There are no pause points between planned steps.

**Why It Exists**: Pause/resume with clean UX requires complex async state management. Direct execution prioritizes determinism and throughput.

**Impact**:
- Risky multi-step operations execute atomically
- No opportunity to review step N before step N+1 executes
- All-or-nothing execution for complex objectives
- Post-hoc review only via inspector and diff

**Constraints for Resolution**:
- Approval points must be explicit in chain structure, not implicit
- Pause must be fail-closed: if user doesn't respond, chain halts
- Resume must re-validate state before continuation
- No automatic timeout-based continuation that bypasses user intent

**Target Direction**: Optional approval checkpoints between chain steps, with explicit halt-on-pause and state re-validation on resume.

---

### 6. No Dynamic Clarification Pauses

**Current State**: The planner cannot request clarification mid-execution. Ambiguous tasks result in best-effort execution or failure.

**Why It Exists**: Dynamic pause/resume requires bidirectional communication infrastructure that doesn't exist. Planner output is strictly one-way: tool calls or completion.

**Impact**:
- Ambiguous requirements result in guessing or conservative failure
- No "did you mean X or Y?" workflow
- Tasks must be fully specified upfront
- Poor outcomes from underspecified objectives

**Constraints for Resolution**:
- Clarification requests must be structured (not free-form chat)
- User response must be integrable into chain state
- Pause/resume mechanics must work for both approval and clarification
- No unbounded back-and-forth that violates execution bounds

**Target Direction**: Structured clarification pauses with bounded response integration into chain continuation.

---

### 7. No Unified Objective Continuity

**Current State**: Plain chat and Forge execution share a transcript but not execution context. Chat does not see what Forge did; Forge does not see chat history as actionable context.

**Why It Exists**: Chat (Ollama) and Forge (planner+tools) are separate subsystems with different contracts. Unified context requires careful state synchronization.

**Impact**:
- User cannot say "now do the same thing to file B" after Forge completes
- Chat advice doesn't automatically inform Forge planning
- No seamless transition from discussion to execution
- Mental model friction: two modes with different capabilities

**Constraints for Resolution**:
- Unified context must respect the tool contract boundary
- Chat suggestions must not bypass validation gates
- Execution context must remain inspectable and deterministic
- No merging of conversational speculation with execution truth

**Target Direction**: Structured context sharing between discussion and execution, with explicit boundaries and validation preservation.

---

### 8. Limited Context Assembly Intelligence

**Current State**: Context assembly is heuristic-based: README, Cargo.toml, src/**/*.rs, *.md. No import graph analysis, no dependency-aware prioritization.

**Why It Exists**: Intelligent context assembly requires parsing and analysis infrastructure that was deferred in favor of explicit user provision.

**Impact**:
- Irrelevant files may consume token budget
- Critical dependencies may be excluded
- No automatic inclusion of transitively imported modules
- Manual file list management for complex changes

**Constraints for Resolution**:
- Assembly must be deterministic (same input → same context)
- Token budgeting must be explicit and configurable
- User must be able to override automatic selection
- No infinite recursion in dependency chasing

**Target Direction**: Relevance-ranked context assembly with import graph analysis and explicit token budgets.

---

## Implementation Gaps

### 9. Runtime-Internal Discovery Tools Not Planner-Visible

**Current State**: `list_dir`, `grep_search`, `execute_command` exist in the tool registry but are not exposed to the planner. They are runtime-internal only.

**Why It Exists**: Tool visibility was restricted to the minimal viable surface for initial release. Broader exposure requires bounded execution guarantees.

**Impact**:
- Planner cannot self-direct exploration
- All file paths must come from user or initial snapshot
- No runtime adaptation to discovered structure

**Resolution Path**: Graduated tool exposure with execution-mode gates. Read-only discovery available in `Analysis` mode only.

---

### 10. Lint Stage Not Implemented

**Current State**: Validation engine skips the lint stage. Syntax, build, and test run; clippy/eslint do not.

**Why It Exists**: Lint configuration is project-specific and complex to auto-detect. Syntax/build/test were prioritized.

**Impact**:
- Style issues pass validation
- Manual linting required post-task

**Resolution Path**: Project-type detection with standard lint command defaults.

---

### 11. No Deterministic Session Replay

**Current State**: Same task may produce different results across runs due to quantization variance, hardware differences, and Ollama version changes.

**Why It Exists**: True determinism requires model-level guarantees that local quantized inference cannot provide.

**Impact**:
- Cannot reliably regression-test planner behavior
- Reproducibility is approximate, not guaranteed

**Constraints for Resolution**:
- Temperature and seed help but cannot eliminate variance
- Replay may require model version pinning
- Full determinism may be impossible with local quantized models

**Resolution Path**: Best-effort determinism with clear documentation of variance sources.

---

## Non-Negotiable Invariants

The following constraints are **fixed** regardless of autonomy expansion. They define the boundary of acceptable implementation.

### Local-First Operation
- Ollama API on localhost only
- No required external API calls (OpenAI, Anthropic, etc.)
- Optional remote Ollama may be supported, but never required
- No telemetry or analytics transmission

### Validation-Gated Persistence
- No file mutation without passing validation
- Syntax → Build → Test stages mandatory for all mutations
- Auto-revert on validation failure (configurable but default-on)
- No bypass mechanisms for "quick fixes"

### Bounded Execution
- Hard iteration limits per step (default 10, configurable with ceiling)
- Hard chain length limits (TBD, but finite)
- Timeout bounds on all external operations
- No infinite loops, no unbounded recursion

### Explicit Auditability
- Every tool call logged with full arguments
- Every mutation recorded with before/after hashes
- Chain state human-inspectable (JSON, not binary)
- Complete execution history reconstructible from logs

### No Silent Background Mutation
- No daemon mode
- No background execution without an explicit user or goal trigger
- No scheduled tasks, no "watch mode" that auto-runs
- Execution tied to explicit user input: task-like plain text, `/goal`, `/task`, or equivalent command

### No Cloud Dependency as Requirement
- Core functionality works with local models only
- Cloud features (if any) strictly optional
- Offline operation must remain fully functional

These invariants exist to preserve the core value proposition: **trustworthy, inspectable, local-only AI assistance** even as autonomy increases.

---

### 12. Snapshot-Based Context Limitations

**Current State**: `RepoSnapshot` provides selective file content (README, Cargo.toml, src/**/*.rs, *.md). It is incomplete by design and token-budget constrained.

**Why It Exists**: Full repo indexing exceeds context limits and reduces determinism. Selective inclusion keeps context focused.

**Impact**:
- Planner only "sees" explicitly included files
- Newly created files post-snapshot are invisible unless explicitly read
- File existence claims cannot be verified without explicit `read_file`
- Planner may hallucinate about files not in snapshot

**Constraints for Resolution**:
- Snapshot expansion must respect token budgets
- Directory tree (without content) may be provided for orientation
- File content inclusion requires explicit user request or bounded heuristics
- No unbounded automatic expansion

**Target Direction**: Expandable snapshot with tree view + selective file content, user-controlled expansion.

---

### 13. Read-Before-Write Enforcement

**Current State**: The Read-Before-Write Gate strictly enforces that files must be explicitly read before mutation. This is a hard constraint.

**Gate Logic**:
```
write_file or apply_patch
         │
         ▼
ReadBeforeWriteGate::evaluate()
         │
         ├─► New file (not exists)? ──► ALLOW
         │
         ├─► File in files_read? ──► Check hash
         │         │
         │         └─► Hash match? ──► ALLOW
         │         └─► Hash mismatch? ──► BLOCK (StaleRead)
         │
         └─► File not in files_read? ──► BLOCK (WriteWithoutRead)
```

**Enforcement Checks**:
1. **Existence**: New files bypass gate; existing files require read authority
2. **Read Record**: File must exist in `AgentState.files_read`
3. **Read Scope**: Full read required (partial reads insufficient)
4. **Hash Freshness**: Content hash must match (detects external modifications)

**Failure Classes**:
- `WriteWithoutRead`: Attempting to write file never read
- `InsufficientReadScope`: Partial read (lines only) for full write
- `StaleRead`: File changed after read (hash mismatch)

**Constraint Status**: This is a **non-negotiable invariant**. It will not be removed, only potentially supplemented by automatic discovery that pre-populates `files_read`.

---

### 14. Event Stream Is Backend-Shaped

**Current State**: JSONL events are designed for observability and debugging, not user-friendly narrative. The inspector shows raw technical stages.

**Why It Exists**: Debugging/auditing priority over UX polish. Deterministic, structured events are easier to log and replay.

**Impact**:
- Events require interpretation
- Inspector shows raw technical stages rather than user narrative
- Learning curve for understanding execution state

**Target Direction**: Maintain structured events internally; add presentation layer that translates to user-friendly descriptions without losing precision.

---

### 15. Model Resolution May Fallback

**Current State**: If the requested model is not found, the system falls back to available models silently.

**Why It Exists**: Graceful degradation prevents complete failure when models are unavailable.

**Impact**:
- May not be using the model you configured
- `/model` may show configured vs active discrepancy
- Unexpected behavior changes when models swap

**Target Direction**: Explicit model selection with clear notifications when fallback occurs; option to require exact model match.

---

### 16. No External API Integration

**Current State**: Rasputin connects only to local Ollama. No integration with OpenAI, Anthropic, or other cloud APIs.

**Why It Exists**: Core value proposition of local-first operation: privacy, offline capability, zero external cost.

**Impact**:
- Model quality limited by local hardware capabilities
- No access to frontier models (GPT-4, Claude 3 Opus)
- Hardware constraints on context window and inference speed

**Constraint Status**: **Non-negotiable invariant**. Local-first operation is not a temporary limitation—it is a permanent architectural commitment. Optional remote Ollama may be supported, but cloud APIs as a requirement will not be implemented.

---

## Current vs Target State

| Current Limitation | Target Capability | Key Constraint |
|-------------------|-------------------|----------------|
| Single-step execution only | Bounded multi-step chains | Each step independently validated |
| No chain-aware memory | Resumable chain state | Validated checkpoints only |
| Limited planner discovery | Bounded repo scanning | Token budgets, depth limits |
| No structured chaining | First-class chain primitives | Human-inspectable chain state |
| No intermediate approval | Optional approval checkpoints | Fail-closed pause/resume |
| No dynamic clarification | Structured clarification pauses | Bounded back-and-forth |
| No unified continuity | Structured context sharing | Validation gates preserved |
| Limited context assembly | Relevance-ranked assembly | Deterministic selection |

## Resolution Status (Updated for V1.5)

| Category | Status |
|----------|--------|
| Core safety invariants | **FIXED** — Never negotiable |
| Single-step execution | **RESOLVED V1.5** — Multi-step chains with validation gating implemented |
| Chain-aware memory | **RESOLVED V1.5** — Persistent chains with resume capability |
| Interrupt handling | **RESOLVED V1.5** — /stop with context preservation |
| Risk forecasting | **RESOLVED V1.5** — GitConflict detection and blocking |
| Auto-resume | **RESOLVED V1.5** — Policy-gated autonomous continuation |
| Discovery limitations | **BOUNDARY** — Bounded expansion planned (list_dir, grep_search implemented) |
| Approval/clarification pauses | **PARTIAL V1.5** — Checkpoint structure exists, not wired to hot path |
| Context assembly | **BOUNDARY** — Ranked assembly planned |
| Lint stage | **GAP** — Implementation pending |
| Session replay | **GAP** — Best-effort only |
| Interface cleanup | **DEBT** — Promote or remove |

## Architectural Philosophy

Rasputin is not rejecting autonomy. It is pursuing **bounded autonomy**:

- **Not**: "You can do anything, trust me"
- **But**: "You can do this bounded set of things, each validated, each inspectable, each interruptible"

The shift from single-run execution to controlled agent behavior happens within strict constraints:

1. Every step must be independently verifiable
2. State between steps must be explicit, not implicit
3. User control points must be fail-closed, not timeout-based
4. All execution remains local, bounded, and auditable

The limitations documented here are the current edges of implementation. The invariants documented here are the permanent boundaries of acceptable behavior.

Between those two lies the path forward.

## V1.5 Trust Loop Status

The V1.5 trust loop stabilization is **COMPLETE** (Phase 2):

| Edge | Component | Status |
|------|-----------|--------|
| 2.3 | /stop kills runtime | ✅ Fixed |
| 3.3 | GitConflict blocks + --force | ✅ Fixed |
| 1.5 | Completion cleanup | ✅ Fixed |
| 2.2 | Step tracking | ✅ Fixed |
| 2.1 | Prepared action context | ✅ Fixed |
| 2.4 | /stop when blocked | ✅ Fixed |
| 3.1 | Failed chain explanation | ✅ Fixed |
| 1.3 | Draft chain UX | ✅ Fixed |

The trust loop is now hardened:
- `/plan → /preview → execute → interrupt → resume → complete → replay`
- Any interrupt at any point → clear recovery path
- No silent failures, no stale state confusion

## When Rasputin Is the Right Choice

Use Rasputin when you need:
- **Local-only execution** with no data exfiltration
- **Validation-gated code changes** that cannot break builds
- **Inspectable execution** with full audit trails
- **Bounded behavior** with hard limits on iteration and scope
- **Controlled autonomy** that expands capability without sacrificing safety
- **Interruptible execution** with clear recovery paths
- **Risk forecasting** with critical risk blocking

Consider alternatives if you need:
- Fully autonomous background agents without supervision
- Cloud-based frontier models (GPT-4, Claude 3 Opus)
- Unbounded conversational coding without execution constraints
- Fire-and-forget task delegation

Rasputin is a **controlled autonomous coding system**—not by abandoning constraints, but by building autonomy within them.
