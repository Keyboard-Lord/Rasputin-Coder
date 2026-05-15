# Rasputin Security and Compliance

## Security Architecture

### Threat Model

| Threat | Mitigation |
|--------|------------|
| Arbitrary code execution | Bounded planner-visible tool surface; direct shell execution is not planner-visible |
| File system escape | Repository boundary validation, path traversal checks for file tools |
| Prompt injection | Strict output contract, validation before execution |
| Resource exhaustion | Bounded iterations, timeouts, process limits |
| Data exfiltration | Local-only design, no external APIs |
| State corruption | Integrity hashing, hash chain verification |

## Security Controls

### 0. Natural-Language Routing Is Not a Safety Bypass

Normal Mode accepts phrases such as "clean up this repo", "fix the warnings", "show me the plan", and "continue where you left off". These phrases are routed to existing command, goal, chain, validation, and stop workflows. The natural-language layer does not execute privileged work on its own.

Safety invariants still apply:
- Broad repository changes require a staged plan and may require confirmation.
- Dangerous or destructive wording blocks or requires explicit operator action.
- Follow-ups require active chain or working-memory context.
- Disposable workspace behavior remains report-only until explicit promotion exists.
- Validation failure halts or repairs within chain policy; it does not invent success.

### 1. Bounded Execution

Hard limits prevent runaway agents:
- **Iteration limit**: 10 (default)
- **Repair limit**: 3 retries
- **Timeout**: 30s per planner call
- **Temperature**: 0.0-0.1 (deterministic)

### 2. Repository Boundary Enforcement

Tools enforce repository boundaries:
```rust
// Path validation in tool execution
if !path.starts_with(working_dir) {
    return Err(ForgeError::IoError("Path outside repository".into()));
}
```

### 3. Read-Before-Write

Prevents blind mutations:
```
write_file or apply_patch
         │
         ▼
ReadBeforeWriteGate::check()
         │
         ├─► File in files_read? ──► PASS
         │
         └─► File not read? ──► FAIL
```

### 4. Validation Gates

Mutations only persist after validation:
1. Syntax check (language-specific)
2. Build check (compile/type-check)
3. Test check (test suite)

Auto-revert on validation failure (fail-closed: **enabled by default**, can be disabled via configuration).

## Sandbox Status

Rasputin does not currently implement a true OS/container sandbox. Its current safety model is repository boundary enforcement plus bounded worker execution. File tools are restricted to the attached repository path, worker tasks run in separate processes, command execution is allowlisted/time-bounded, and mutations are validation-gated. This reduces accidental damage but does not provide the same guarantees as chroot, containers, VM isolation, seccomp, App Sandbox, or network namespaces.

`disposable_workspace` improves workspace isolation by running TUI-launched tasks in a temporary git worktree and emitting a promotion report instead of mutating the source workspace. It is still not OS-level containment: commands and build scripts execute with the user's permissions inside the disposable worktree.

Disposable workspace behavior is intentionally narrow:
- Implemented only through TUI-managed execution paths.
- Direct `forge_bootstrap` execution rejects `disposable_workspace`; the TUI creates the worktree and launches the child runtime with `repo_boundary_only`.
- Worktrees are created under the system temporary directory and are rejected if the computed path is outside that root.
- Changed files remain inside the disposable worktree until explicit promotion exists.
- Promotion is report-only. There is no automatic source workspace mutation.
- Source HEAD lookup, diff generation, promotion report generation, child spawn failure, cancellation, and cleanup failure are surfaced as failures.
- `retain_on_failure` and `retain_on_success` preserve worktrees only when explicitly configured.

### 5. Minimal Tool Surface

Planner-visible tools are explicitly registered by runtime policy:
- `read_file` — Information gathering
- `write_file` — File creation
- `apply_patch` — Surgical modification
- `list_dir` — Directory exploration
- `grep_search` — Pattern search
- `dependency_graph` — Bounded dependency inspection
- `symbol_index` — Bounded symbol inspection
- `entrypoint_detector` — Entrypoint discovery
- `lint_runner` — Policy-bounded lint validation
- `test_runner` — Policy-bounded test validation

Direct `execute_command` and `browser_preview` remain in the internal registry but are not exposed to the planner-visible runtime tool list.

### 6. Bounded Worker Execution

- One worker process per task
- Clean termination on completion
- Worker death doesn't corrupt TUI
- No shared memory between components
- Not a security sandbox or arbitrary-code containment boundary

### 7. Local-Only Design (Architecturally Enforced)

**Ollama HTTP Client Constraint**:
```rust
// ollama.rs - enforced at client construction
is_loopback_http_endpoint(endpoint)
```

- **Loopback-only HTTP**: Remote Ollama endpoints are **rejected at runtime**
- **No cloud AI services**: OpenAI, Anthropic, or other cloud APIs are **architecturally inaccessible**
- **LLM client egress is loopback-only**: Rasputin's Ollama client rejects non-loopback endpoints
- **No telemetry/analytics**: Zero data collection or external communication

**Security Invariant**: Even with malicious model configuration, Rasputin's own LLM client cannot call remote endpoints or cloud APIs. This is not a general network sandbox: build scripts, tests, and subprocesses still run with the user's normal OS permissions unless a future OS/container backend is added.

## Compliance

### Privacy

| Aspect | Status |
|--------|--------|
| Code leaves machine | **NO** |
| Cloud AI APIs | **NO** |
| Telemetry/analytics | **NO** |
| Local storage encryption | Not implemented |
| Data retention | User-controlled (local files) |

### Data Protection

**Stored Data**:
- `~/.local/share/rasputin/state.json`: Conversations, repos, settings
- `~/.local/share/rasputin/rasputin.log`: Application logs
- Repository files: As modified by tools

**No Collection Of**:
- Source code (unless locally stored)
- User behavior analytics
- Error reports
- Usage statistics

### Audit Trail

**Governance Logging**:
```rust
// crates/forge-runtime/src/governance.rs
pub struct GovernanceLog {
    pub drift_events: Vec<DriftEvent>,
    pub protocol_validations: Vec<ProtocolValidationDecision>,
    pub mutation_validations: Vec<MutationValidationDecision>,
}
```

**Change History**:
```rust
pub struct ChangeRecord {
    pub iteration: u32,
    pub timestamp: u64,
    pub path: PathBuf,
    pub change_type: ChangeType,
    pub tool_used: String,
    pub description: String,
}
```

## Security Best Practices

### For Users

1. **Review before execution**: Inspect generated goal plans and any manual `/task` commands
2. **Check validation results**: Review inspector validation tab
3. **Use version control**: Commit before major Forge tasks
4. **Verify diffs**: Check diff tab before accepting changes
5. **Monitor logs**: Watch for unexpected behavior

### For Developers

1. **Fail-closed**: Return errors rather than proceeding
2. **Validate inputs**: Tool arguments, paths, content
3. **Hash verification**: State integrity, content verification
4. **No secrets in logs**: Don't log sensitive content
5. **Path sanitization**: Normalize and validate all paths

## Vulnerability Handling

### Known Limitations (Security-Related)

| Limitation | Risk | Mitigation |
|------------|------|------------|
| No per-action approval | Unattended execution | Post-hoc review, validation gates, approval checkpoints |
| Model hallucination | Incorrect tool usage | Bounded execution, validation |
| Local model quality | Variable output | Temperature clamp, repair loop |
| No encrypted storage | Local data exposure | File system permissions |
| Dirty worktree | Conflicts with uncommitted changes | Git grounding warnings |
| Critical risks | Execution may fail | Risk preview blocks execution |

### Reporting

Security issues should be reported through:
- Repository Issues (if public)
- Direct maintainer contact (if private)

## Security Checklist

### Pre-Task
- [ ] Workspace attached correctly
- [ ] Task description reviewed
- [ ] No sensitive data in task

### Post-Task
- [ ] Validation passed
- [ ] Diff reviewed
- [ ] Files inspected
- [ ] No unexpected changes

### Operational
- [ ] Ollama on localhost only
- [ ] Model from trusted source
- [ ] Logs monitored
- [ ] State backed up

## Compliance Standards

| Standard | Status | Notes |
|----------|--------|-------|
| GDPR | Not applicable | No personal data collection |
| CCPA | Not applicable | No personal data collection |
| SOC 2 | Not certified | Local-only, no service |
| ISO 27001 | Not certified | Local-only deployment |

## V1.5 Security Features

### Risk Forecasting
Before chain execution, risks are detected and classified:
- GitConflict — Critical risk, blocks execution
- ValidationFailure — Warning, does not block
- MissingContext — Caution, does not block

### Git Grounding
Repository state captured before execution:
- Branch name and commit hash
- Dirty worktree detection
- Warning on uncommitted changes

### Approval Checkpoints
Human review at execution boundaries:
- PreExecution — Before chain starts
- PreMutationCommit — Before changes persist
- PostValidationPreAdvance — After validation
- ReplayMismatchReview — When replay diverges

## Security Roadmap

**Not Implemented** (by design):
- End-to-end encryption for persistence
- Multi-user access control
- Audit log export
- Security scanning integration

**Future Considerations**:
- Signed model verification
- Encrypted state storage
- Fine-grained approval policies
- Multi-factor checkpoint approvals
