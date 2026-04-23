//! FORGE Conformance Tests
//!
//! End-to-end tests that prove code matches documented Rasputin rules.
//! Per FORGE_PLANNER_OUTPUT_CONTRACT_SPEC.md Section 14.6.

#[cfg(test)]
mod tests {
    use crate::planner::output_adapter::{AdapterResult, CanonicalOutputAdapter};
    use crate::planner::protocol_validator::{
        PlannerProtocolValidator, ValidationContext, ValidationDecision, ValidationFailureClass,
    };
    use crate::runtime_gates::{
        CompletionGate, CompletionGateResult, ReadBeforeWriteGate, ReadBeforeWriteResult,
        normalize_path,
    };
    use crate::state::AgentState;
    use crate::types::{CompletionReason, ExecutionMode, FileRecord, PlannerOutput};
    use std::path::PathBuf;

    fn test_context(available_tools: &[&str]) -> ValidationContext {
        ValidationContext {
            available_tools: available_tools
                .iter()
                .map(|tool| (*tool).to_string())
                .collect(),
            mode: ExecutionMode::Edit,
            has_pending_operations: false,
            files_read: vec![],
            iteration: 1,
            tool_calls_executed: 0,
            pending_validation: vec![],
            known_errors: vec![],
            task_description: "test task".to_string(),
            read_records: vec![],
            allow_partial_read_writes: false,
        }
    }

    // ============================================================================
    // TEST: Planner Output Protocol
    // ============================================================================

    #[test]
    fn test_reject_shorthand_tool_output() {
        let validator = PlannerProtocolValidator::new();
        let context = test_context(&["read_file", "write_file"]);

        // Shorthand form should be rejected
        let shorthand = r#"{"action":"read_file","path":"src/main.rs"}"#;
        let result = validator.validate(shorthand, &context);

        match &result.decision {
            ValidationDecision::Reject { failure_class, .. } => {
                assert!(
                    matches!(
                        failure_class,
                        ValidationFailureClass::ShorthandDetected
                            | ValidationFailureClass::MissingRequiredField
                            | ValidationFailureClass::SchemaViolation
                    ),
                    "Expected shorthand detection, got {:?}",
                    failure_class
                );
            }
            _ => panic!(
                "Expected rejection for shorthand, got {:?}",
                result.decision
            ),
        }
    }

    #[test]
    fn test_reject_wrapper_schema() {
        let validator = PlannerProtocolValidator::new();
        let context = test_context(&["read_file"]);

        // Wrapper schema should be rejected
        let wrapper = r#"{"status":"ready","plan":[],"action":{"type":"tool_call","tool_call":{"name":"read_file","arguments":{}}}}"#;
        let result = validator.validate(wrapper, &context);

        assert!(
            matches!(
                &result.decision,
                ValidationDecision::Reject { .. } | ValidationDecision::Escalate { .. }
            ),
            "Wrapper schema should be rejected, got {:?}",
            result.decision
        );
    }

    #[test]
    fn test_reject_leading_prose() {
        let validator = PlannerProtocolValidator::new();
        let context = test_context(&["read_file"]);

        // Leading prose should cause JSON parse failure
        let prose = "I'll read the file now.\n\n{\"type\":\"tool_call\",\"tool_call\":{\"name\":\"read_file\",\"arguments\":{\"path\":\"test.txt\"}}}";
        let result = validator.validate(prose, &context);

        assert!(
            !matches!(result.decision, ValidationDecision::Accept),
            "Leading prose should prevent acceptance"
        );
    }

    #[test]
    fn test_reject_trailing_prose() {
        let validator = PlannerProtocolValidator::new();
        let context = test_context(&["read_file"]);

        // Trailing prose should be rejected
        let trailing = "{\"type\":\"tool_call\",\"tool_call\":{\"name\":\"read_file\",\"arguments\":{\"path\":\"test.txt\"}}}\n\nHope this helps!";
        let result = validator.validate(trailing, &context);

        assert!(
            !matches!(result.decision, ValidationDecision::Accept),
            "Trailing prose should prevent acceptance"
        );
    }

    #[test]
    fn test_reject_fake_idle_action() {
        let validator = PlannerProtocolValidator::new();
        let context = test_context(&["read_file", "write_file"]);

        // Idle/noop action should be rejected
        let idle = r#"{"type":"tool_call","tool_call":{"name":"idle","arguments":{}}}"#;
        let result = validator.validate(idle, &context);

        match &result.decision {
            ValidationDecision::Reject { failure_class, .. } => {
                assert!(
                    matches!(
                        failure_class,
                        ValidationFailureClass::UnknownTool | ValidationFailureClass::IdleAction
                    ),
                    "Expected unknown tool detection for idle, got {:?}",
                    failure_class
                );
            }
            _ => panic!(
                "Expected rejection for idle action, got {:?}",
                result.decision
            ),
        }
    }

    #[test]
    fn test_accept_valid_canonical_tool_call() {
        let validator = PlannerProtocolValidator::new();
        let context = test_context(&["read_file"]);

        // Valid canonical form should be accepted
        let valid = r#"{"type":"tool_call","tool_call":{"name":"read_file","arguments":{"path":"src/main.rs"}}}"#;
        let result = validator.validate(valid, &context);

        assert!(
            matches!(result.decision, ValidationDecision::Accept),
            "Valid canonical tool_call should be accepted, got {:?}",
            result.decision
        );
    }

    // ============================================================================
    // TEST: Completion Gate
    // ============================================================================

    #[test]
    fn test_completion_gate_rejects_no_tool_execution() {
        let state = AgentState::new(10, "test".to_string(), crate::types::ExecutionMode::Edit);
        let reason = CompletionReason::new("Task is complete");

        let result = CompletionGate::evaluate(&reason, &state, false, &[]);

        match result {
            CompletionGateResult::Reject { failure_class, .. } => {
                assert_eq!(
                    failure_class,
                    ValidationFailureClass::CompletionWithoutEvidence,
                    "Should reject completion without tool execution"
                );
            }
            _ => panic!("Expected rejection for completion without evidence"),
        }
    }

    #[test]
    fn test_completion_gate_rejects_vague_reason() {
        let mut state = AgentState::new(10, "test".to_string(), crate::types::ExecutionMode::Edit);
        // Simulate some execution
        state.iteration = 1;

        let reason = CompletionReason::new("Looks good");
        let result = CompletionGate::evaluate(&reason, &state, false, &[]);

        match result {
            CompletionGateResult::Reject { failure_class, .. } => {
                assert_eq!(
                    failure_class,
                    ValidationFailureClass::VagueCompletionReason,
                    "Should reject vague completion reason"
                );
            }
            _ => panic!("Expected rejection for vague reason"),
        }
    }

    #[test]
    fn test_completion_gate_rejects_pending_validation() {
        let mut state = AgentState::new(10, "test".to_string(), crate::types::ExecutionMode::Edit);
        state.iteration = 1;

        let reason = CompletionReason::new("src/main.rs contains fn main() at line 12");
        let result = CompletionGate::evaluate(&reason, &state, true, &[]);

        match result {
            CompletionGateResult::Reject { failure_class, .. } => {
                assert_eq!(
                    failure_class,
                    ValidationFailureClass::CompletionWithPendingValidation,
                    "Should reject completion with pending validation"
                );
            }
            _ => panic!("Expected rejection for pending validation"),
        }
    }

    #[test]
    fn test_completion_gate_rejects_known_errors() {
        let mut state = AgentState::new(10, "test".to_string(), crate::types::ExecutionMode::Edit);
        state.iteration = 1;

        let reason = CompletionReason::new("src/main.rs contains fn main()");
        let known_errors = vec!["Syntax error in src/main.rs".to_string()];
        let result = CompletionGate::evaluate(&reason, &state, false, &known_errors);

        match result {
            CompletionGateResult::Reject { failure_class, .. } => {
                assert_eq!(
                    failure_class,
                    ValidationFailureClass::CompletionWithKnownErrors,
                    "Should reject completion with known errors"
                );
            }
            _ => panic!("Expected rejection for known errors"),
        }
    }

    #[test]
    fn test_completion_gate_accepts_valid_reason() {
        let mut state = AgentState::new(10, "test".to_string(), crate::types::ExecutionMode::Edit);
        state.iteration = 1;

        let reason = CompletionReason::new("src/main.rs contains fn main() at line 12");
        let result = CompletionGate::evaluate(&reason, &state, false, &[]);

        assert!(
            matches!(result, CompletionGateResult::Accept),
            "Should accept valid state-justified completion"
        );
    }

    // ============================================================================
    // TEST: Read-Before-Write Gate
    // ============================================================================

    #[test]
    fn test_read_before_write_allows_new_file() {
        let state = AgentState::new(10, "test".to_string(), crate::types::ExecutionMode::Edit);
        let path = PathBuf::from("new_file.txt");

        // New file (doesn't exist) should be allowed without prior read
        let result = ReadBeforeWriteGate::evaluate(&path, false, &state, None);

        assert!(
            matches!(result, ReadBeforeWriteResult::Allow),
            "New file creation should be allowed"
        );
    }

    #[test]
    fn test_read_before_write_blocks_without_read() {
        let state = AgentState::new(10, "test".to_string(), crate::types::ExecutionMode::Edit);
        let path = PathBuf::from("existing.txt");

        // Existing file without read record should be blocked
        let result = ReadBeforeWriteGate::evaluate(&path, true, &state, None);

        match result {
            ReadBeforeWriteResult::Block { failure_class, .. } => {
                assert_eq!(
                    failure_class,
                    ValidationFailureClass::WriteWithoutRead,
                    "Should block write without prior read"
                );
            }
            _ => panic!("Expected block for write without read"),
        }
    }

    #[test]
    fn test_read_before_write_blocks_partial_read() {
        let mut state = AgentState::new(10, "test".to_string(), crate::types::ExecutionMode::Edit);
        let path = PathBuf::from("existing.txt");

        // Record a partial read
        let file_record = FileRecord::new(&path, "content", Some((1, 10)), 1);
        state.record_file_read(file_record);

        let result = ReadBeforeWriteGate::evaluate(&path, true, &state, None);

        match result {
            ReadBeforeWriteResult::Block { failure_class, .. } => {
                assert_eq!(
                    failure_class,
                    ValidationFailureClass::InsufficientReadScope,
                    "Should block write after partial read"
                );
            }
            _ => panic!("Expected block for partial read"),
        }
    }

    #[test]
    fn test_read_before_write_blocks_stale_hash() {
        let mut state = AgentState::new(10, "test".to_string(), crate::types::ExecutionMode::Edit);
        let path = PathBuf::from("existing.txt");

        // Record a full read with hash
        let file_record = FileRecord::new(&path, "original content", None, 1);
        state.record_file_read(file_record);

        // Current content is different
        let result = ReadBeforeWriteGate::evaluate(&path, true, &state, Some("modified content"));

        match result {
            ReadBeforeWriteResult::Block { failure_class, .. } => {
                assert_eq!(
                    failure_class,
                    ValidationFailureClass::StaleRead,
                    "Should block write when file changed after read"
                );
            }
            _ => panic!("Expected block for stale read"),
        }
    }

    #[test]
    fn test_read_before_write_allows_valid_read_then_write() {
        let mut state = AgentState::new(10, "test".to_string(), crate::types::ExecutionMode::Edit);
        let path = PathBuf::from("existing.txt");

        // Record a full read
        let file_record = FileRecord::new(&path, "current content", None, 1);
        state.record_file_read(file_record);

        // Same content - hash matches
        let result = ReadBeforeWriteGate::evaluate(&path, true, &state, Some("current content"));

        assert!(
            matches!(result, ReadBeforeWriteResult::Allow),
            "Should allow write with valid read authority and matching hash"
        );
    }

    // ============================================================================
    // TEST: Policy B Normalization
    // ============================================================================

    #[test]
    fn test_normalize_shorthand_completion() {
        let adapter = CanonicalOutputAdapter::new();
        let context = test_context(&["read_file"]);

        let shorthand = r#"{"action":"complete","justification":"src/main.rs contains fn main()"}"#;
        let result = adapter.process(shorthand, &context);

        match result {
            AdapterResult::Normalized { action, output } => {
                assert_eq!(action.drift_pattern, "shorthand_completion");
                match output {
                    PlannerOutput::Completion { reason } => {
                        assert_eq!(reason.as_str(), "src/main.rs contains fn main()");
                    }
                    _ => panic!("Expected completion output"),
                }
            }
            AdapterResult::Success(_) => {
                // Also acceptable if validator already accepts it
            }
            _ => panic!(
                "Expected normalization for shorthand completion, got {:?}",
                result
            ),
        }
    }

    #[test]
    fn test_normalize_shorthand_tool_call() {
        let adapter = CanonicalOutputAdapter::new();
        let context = test_context(&["read_file"]);

        let shorthand = r#"{"action":"read_file","path":"src/main.rs"}"#;
        let result = adapter.process(shorthand, &context);

        match result {
            AdapterResult::Normalized { action, output } => {
                assert_eq!(action.drift_pattern, "shorthand_tool_call");
                match output {
                    PlannerOutput::ToolCall(tc) => {
                        assert_eq!(tc.name.as_str(), "read_file");
                    }
                    _ => panic!("Expected tool call output"),
                }
            }
            AdapterResult::Success(_) => {
                // Also acceptable if validator already accepts it
            }
            _ => panic!(
                "Expected normalization for shorthand tool call, got {:?}",
                result
            ),
        }
    }

    #[test]
    fn test_reject_ambiguous_output() {
        let adapter = CanonicalOutputAdapter::new();
        let context = test_context(&["read_file"]);

        // Ambiguous output - "result" field could mean multiple things
        let ambiguous = r#"{"result":"success"}"#;
        let result = adapter.process(ambiguous, &context);

        assert!(
            matches!(
                result,
                AdapterResult::Reject { .. } | AdapterResult::Escalate { .. }
            ),
            "Ambiguous output should be rejected, got {:?}",
            result
        );
    }

    // ============================================================================
    // TEST: Path Normalization
    // ============================================================================

    #[test]
    fn test_normalize_path_handles_relative() {
        let path = PathBuf::from("./src/../src/main.rs");
        let normalized = normalize_path(&path);

        // Should collapse to src/main.rs
        assert!(!normalized.to_string_lossy().contains(".."));
        assert!(!normalized.to_string_lossy().contains("./"));
    }

    #[test]
    fn test_normalize_path_preserves_absolute() {
        let path = PathBuf::from("/home/user/project/src/main.rs");
        let normalized = normalize_path(&path);

        assert!(normalized.is_absolute() || normalized.starts_with("/"));
    }

    // ============================================================================
    // TEST: Exact Schema Enforcement
    // ============================================================================

    #[test]
    fn test_reject_unknown_top_level_field() {
        let validator = PlannerProtocolValidator::new();
        let context = test_context(&["read_file"]);

        // Extra field at top level should be rejected
        let extra_field = r#"{"type":"completion","reason":"Done","summary":"All files updated"}"#;
        let result = validator.validate(extra_field, &context);

        assert!(
            !matches!(result.decision, ValidationDecision::Accept),
            "Unknown top-level field should be rejected"
        );
    }

    #[test]
    fn test_reject_unknown_nested_field() {
        let validator = PlannerProtocolValidator::new();
        let context = test_context(&["read_file"]);

        // Extra field inside tool_call should be rejected
        let extra_nested = r#"{"type":"tool_call","tool_call":{"name":"read_file","arguments":{},"extra":"value"}}"#;
        let result = validator.validate(extra_nested, &context);

        assert!(
            !matches!(result.decision, ValidationDecision::Accept),
            "Unknown nested field should be rejected"
        );
    }

    // ============================================================================
    // TEST: Discovery Tools Are Planner-Visible
    // ============================================================================

    #[test]
    fn test_list_dir_tool_is_planner_visible() {
        let validator = PlannerProtocolValidator::new();
        let context = test_context(&["list_dir", "read_file"]);

        let list_dir_call =
            r#"{"type":"tool_call","tool_call":{"name":"list_dir","arguments":{"path":"src"}}}"#;
        let result = validator.validate(list_dir_call, &context);

        assert!(
            matches!(result.decision, ValidationDecision::Accept),
            "list_dir should be accepted when in available tools, got {:?}",
            result.decision
        );
    }

    #[test]
    fn test_grep_search_tool_is_planner_visible() {
        let validator = PlannerProtocolValidator::new();
        let context = test_context(&["grep_search", "read_file"]);

        let grep_call = r#"{"type":"tool_call","tool_call":{"name":"grep_search","arguments":{"query":"fn main","path":"src"}}}"#;
        let result = validator.validate(grep_call, &context);

        assert!(
            matches!(result.decision, ValidationDecision::Accept),
            "grep_search should be accepted when in available tools, got {:?}",
            result.decision
        );
    }

    #[test]
    fn test_discovery_tools_not_visible_without_context() {
        let validator = PlannerProtocolValidator::new();
        let context = test_context(&["read_file"]); // list_dir not in available tools

        let list_dir_call =
            r#"{"type":"tool_call","tool_call":{"name":"list_dir","arguments":{"path":"src"}}}"#;
        let result = validator.validate(list_dir_call, &context);

        assert!(
            !matches!(result.decision, ValidationDecision::Accept),
            "list_dir should be rejected when not in available tools"
        );
    }
}
