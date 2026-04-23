# Rasputin Security and Compliance

## Security Architecture

### Threat Model

| Threat | Mitigation |
|--------|------------|
| Arbitrary code execution | Minimal tool surface, no shell in planner tools |
| File system escape | Repository boundary validation, path traversal checks |
| Prompt injection | Strict output contract, validation before execution |
| Resource exhaustion | Bounded iterations, timeouts, process limits |
| Data exfiltration | Local-only design, no external APIs |
| State corruption | Integrity hashing, hash chain verification |

## Security Controls

### 1. Bounded Execution

Hard limits prevent runaway agents:
- **Iteration limit**: 10 (default)
- **Repair limit**: 3 retries
- **Timeout**: 30s per planner call
- **Temperature**: 0.0-0.1 (deterministic)

### 2. Repository Sandboxing

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

### 5. Minimal Tool Surface

Planner sees only 5 tools:
- `read_file` — Information gathering
- `write_file` — File creation
- `apply_patch` — Surgical modification
- `list_dir` — Directory exploration
- `grep_search` — Pattern search

No `execute_command` for planner (TUI-only).

### 6. Process Isolation

- One worker process per task
- Clean termination on completion
- Worker death doesn't corrupt TUI
- No shared memory between components

### 7. Local-Only Design (Architecturally Enforced)

**Ollama HTTP Client Constraint**:
```rust
// ollama.rs - enforced at client construction
assert!(
    endpoint.starts_with("http://127.0.0.1:")
        || endpoint.starts_with("http://[::1]:")
        || endpoint.starts_with("http://localhost:"),
    "Ollama endpoint must be loopback-only"
);
```

- **Loopback-only HTTP**: Remote Ollama endpoints are **rejected at runtime**
- **No cloud AI services**: OpenAI, Anthropic, or other cloud APIs are **architecturally inaccessible**
- **No network egress**: Except loopback Ollama calls
- **No telemetry/analytics**: Zero data collection or external communication

**Security Invariant**: Even with malicious configuration, the system cannot call remote endpoints or cloud APIs.

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
