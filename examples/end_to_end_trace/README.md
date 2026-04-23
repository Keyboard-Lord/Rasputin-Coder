# End-to-End Trace Example

This directory contains a reproducible artifact of the "create a rust CLI that prints hello world" task.

## Quick Run

```bash
# From repo root
./rasputin /tmp/hello_test

# In TUI:
/task create a rust CLI that prints hello world
```

## Expected Artifacts

| File | Description |
|------|-------------|
| `main.rs` | Generated Rust source |
| `Cargo.toml` | Generated project manifest |
| `trace.jsonl` | Full JSONL event log (if captured) |
| `state_view_iter_0.json` | StateView sent to planner, iteration 0 |
| `planner_output_iter_0.json` | Planner raw output, iteration 0 |
| `planner_output_iter_1.json` | Planner raw output, iteration 1 |
| `planner_output_iter_2.json` | Planner raw output (completion) |

## Validation Steps

After the task completes:

```bash
cd /tmp/hello_test
cat main.rs
# Should show: fn main() { println!("hello world"); }

cat Cargo.toml
# Should show: [package] name = "hello" ...

rustc --emit=metadata main.rs
# Should exit 0 (syntax validation)

cargo run
# Should print: hello world
```

## Event Verification

Key events that must appear in JSONL trace:

1. `ITERATION_START` (iteration 0)
2. `PREFLIGHT_PASSED`
3. `PLANNER_OUTPUT` (tool_call write_file)
4. `PROTOCOL_VALIDATION_ACCEPT`
5. `TOOL_EXECUTING` (write_file)
6. `TOOL_RESULT` (success)
7. `VALIDATION_STAGE` (syntax PASS)
8. `STATE_COMMIT`
9. `ITERATION_START` (iteration 1)
10. `ITERATION_START` (iteration 2)
11. `COMPLETION_ACCEPT`
12. `RUNTIME_COMPLETE`

## Failure Modes (for testing)

### To test validation failure:
```
/task create a rust file with syntax errors
```

Expected: `VALIDATION_STAGE` with `syntax FAIL`, auto-revert if enabled.

### To test read-before-write failure:
```
/task apply a patch to src/nonexistent.rs without reading it
```

Expected: `READ_BEFORE_WRITE_FAIL`, repair loop active.

### To test planner failure:
```
/task [extremely ambiguous task that confuses the model]
```

Expected: `PROTOCOL_VALIDATION_REJECT` or `REPAIR_LOOP_EXHAUSTED`.

## Reference: StateView Schema

```json
{
  "task": "string",
  "session_id": "forge-<timestamp>-<counter>",
  "iteration": 0,
  "max_iterations": 10,
  "files_read": [],
  "files_written": [],
  "available_tools": ["read_file", "write_file", "apply_patch"],
  "recent_executions": [],
  "recent_errors": [],
  "mode": "Edit"
}
```

## Reference: Planner Output Schema (Tool Call)

```json
{
  "tool_call": {
    "tool": "write_file",
    "args": {
      "path": "main.rs",
      "content": "fn main() {\n    println!(\"hello world\");\n}"
    }
  }
}
```

## Reference: Planner Output Schema (Completion)

```json
{
  "completion": {
    "reason": "Created complete Rust CLI project with main.rs and Cargo.toml"
  }
}
```

## Success Criteria

This trace is successful if:
- 2 files created (main.rs, Cargo.toml)
- Syntax validation passes for both
- Completion accepted
- Total iterations ≤ 3
- Total time < 10 seconds
