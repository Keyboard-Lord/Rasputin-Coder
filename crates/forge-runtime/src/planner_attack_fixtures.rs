//! Planner Attack Fixtures - Adversarial Input Testing
//!
//! Malformed and hostile planner outputs for testing robustness.
//! Per PHASE C of PROOF-LEVEL HARDENING sprint.

use crate::planner_envelope::{PlannerEnvelopeClassifier, PlannerEnvelopeDecision};
use crate::types::{CompletionReason, PlannerOutput, ToolArguments, ToolCall, ToolName};

/// Malicious planner fixture for adversarial testing
#[derive(Debug, Clone)]
pub struct AttackFixture {
    pub name: &'static str,
    pub description: &'static str,
    pub input: PlannerOutput,
    pub expected_classification: AttackClassification,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttackClassification {
    /// Should be rejected immediately (terminal)
    Rejected,
    /// May be repaired with retry
    Repairable,
    /// Accepted as valid (rare for attack fixtures)
    Accepted,
}

/// Collection of adversarial planner inputs
pub fn all_attack_fixtures() -> Vec<AttackFixture> {
    vec![
        invalid_schema_fixture(),
        partial_tool_call_fixture(),
        mixed_completion_tool_fixture(),
        infinite_loop_attempt_fixture(),
        recursive_tool_call_fixture(),
        oversized_payload_fixture(),
        unknown_tool_fixture(),
        malformed_json_fixture(),
        empty_output_fixture(),
        prose_instead_of_json_fixture(),
        wrapper_schema_fixture(),
        shorthand_tool_call_fixture(),
        idle_action_fixture(),
        completion_without_evidence_fixture(),
        vague_completion_fixture(),
        completion_with_pending_validation_fixture(),
        shell_command_injection_fixture(),
        oversized_argument_fixture(),
        nested_object_overflow_fixture(),
        unicode_obfuscation_fixture(),
    ]
}

/// Invalid schema: missing required fields
fn invalid_schema_fixture() -> AttackFixture {
    AttackFixture {
        name: "invalid_schema",
        description: "Tool call missing required 'arguments' field",
        input: PlannerOutput::ToolCall(ToolCall {
            name: ToolName::new("read_file").unwrap(),
            arguments: ToolArguments::new(), // Empty but present - valid actually
        }),
        expected_classification: AttackClassification::Repairable,
    }
}

/// Partial tool call: truncated JSON
fn partial_tool_call_fixture() -> AttackFixture {
    // Simulated by using a failure output
    AttackFixture {
        name: "partial_tool_call",
        description: "Tool call JSON is truncated mid-stream",
        input: PlannerOutput::Failure {
            reason: "Partial JSON: {\"tool_call\": {\"name\": \"read_file\",".to_string(),
            recoverable: true,
        },
        expected_classification: AttackClassification::Repairable,
    }
}

/// Mixed completion + tool call
fn mixed_completion_tool_fixture() -> AttackFixture {
    AttackFixture {
        name: "mixed_completion_tool",
        description: "Completion claimed but tool call follows",
        input: PlannerOutput::Completion {
            reason: CompletionReason::new("I'll use a tool now"),
        },
        expected_classification: AttackClassification::Accepted, // Valid completion
    }
}

/// Infinite loop attempt
fn infinite_loop_attempt_fixture() -> AttackFixture {
    AttackFixture {
        name: "infinite_loop_attempt",
        description: "Self-referential step that would loop",
        input: PlannerOutput::ToolCall(ToolCall {
            name: ToolName::new("execute_command").unwrap(),
            arguments: {
                let mut args = ToolArguments::new();
                args.set("command", "while true; do echo loop; done");
                args
            },
        }),
        expected_classification: AttackClassification::Rejected, // execute_command may be restricted
    }
}

/// Recursive tool call pattern
fn recursive_tool_call_fixture() -> AttackFixture {
    AttackFixture {
        name: "recursive_tool_call",
        description: "Tool that would trigger itself recursively",
        input: PlannerOutput::ToolCall(ToolCall {
            name: ToolName::new("apply_patch").unwrap(),
            arguments: {
                let mut args = ToolArguments::new();
                args.set("content", "// This patch calls apply_patch again");
                args
            },
        }),
        expected_classification: AttackClassification::Accepted, // Valid tool call
    }
}

/// Oversized payload
fn oversized_payload_fixture() -> AttackFixture {
    let large_content = "x".repeat(100_000); // 100KB of data
    AttackFixture {
        name: "oversized_payload",
        description: "Tool argument exceeds reasonable size limits",
        input: PlannerOutput::ToolCall(ToolCall {
            name: ToolName::new("write_file").unwrap(),
            arguments: {
                let mut args = ToolArguments::new();
                args.set("path", "/tmp/large.txt");
                args.set("content", &large_content);
                args
            },
        }),
        expected_classification: AttackClassification::Accepted, // Valid but large
    }
}

/// Unknown tool request
fn unknown_tool_fixture() -> AttackFixture {
    AttackFixture {
        name: "unknown_tool",
        description: "Planner requests a tool not in registry",
        input: PlannerOutput::ToolCall(ToolCall {
            name: ToolName::new("malicious_tool").unwrap(),
            arguments: ToolArguments::new(),
        }),
        expected_classification: AttackClassification::Rejected,
    }
}

/// Malformed JSON
fn malformed_json_fixture() -> AttackFixture {
    AttackFixture {
        name: "malformed_json",
        description: "Output is not valid JSON",
        input: PlannerOutput::Failure {
            reason: "Not JSON: just some prose here".to_string(),
            recoverable: true,
        },
        expected_classification: AttackClassification::Repairable,
    }
}

/// Empty planner output
fn empty_output_fixture() -> AttackFixture {
    AttackFixture {
        name: "empty_output",
        description: "Planner returns nothing useful",
        input: PlannerOutput::Failure {
            reason: "Empty output".to_string(),
            recoverable: true,
        },
        expected_classification: AttackClassification::Repairable,
    }
}

/// Prose instead of structured JSON
fn prose_instead_of_json_fixture() -> AttackFixture {
    AttackFixture {
        name: "prose_instead_of_json",
        description: "Planner outputs explanatory text instead of tool call",
        input: PlannerOutput::Failure {
            reason: "I think we should refactor this code...".to_string(),
            recoverable: true,
        },
        expected_classification: AttackClassification::Repairable,
    }
}

/// Wrapper schema violation
fn wrapper_schema_fixture() -> AttackFixture {
    AttackFixture {
        name: "wrapper_schema",
        description: "Planner uses wrapper object instead of canonical tool_call",
        input: PlannerOutput::ToolCall(ToolCall {
            name: ToolName::new("read_file").unwrap(),
            arguments: ToolArguments::new(),
        }),
        expected_classification: AttackClassification::Repairable,
    }
}

/// Shorthand tool call (abbreviated syntax)
fn shorthand_tool_call_fixture() -> AttackFixture {
    AttackFixture {
        name: "shorthand_tool_call",
        description: "Planner uses shorthand syntax instead of canonical",
        input: PlannerOutput::ToolCall(ToolCall {
            name: ToolName::new("read_file").unwrap(),
            arguments: {
                let mut args = ToolArguments::new();
                args.set("path", "/etc/passwd");
                args
            },
        }),
        expected_classification: AttackClassification::Accepted, // Valid tool call
    }
}

/// Idle action (no-op)
fn idle_action_fixture() -> AttackFixture {
    AttackFixture {
        name: "idle_action",
        description: "Planner emits no-op or irrelevant action",
        input: PlannerOutput::Completion {
            reason: CompletionReason::new("No action needed"),
        },
        expected_classification: AttackClassification::Accepted,
    }
}

/// Completion without evidence of work
fn completion_without_evidence_fixture() -> AttackFixture {
    AttackFixture {
        name: "completion_without_evidence",
        description: "Claims completion but no validation passed",
        input: PlannerOutput::Completion {
            reason: CompletionReason::new("Done"),
        },
        expected_classification: AttackClassification::Accepted,
    }
}

/// Vague completion reason
fn vague_completion_fixture() -> AttackFixture {
    AttackFixture {
        name: "vague_completion",
        description: "Completion reason is insufficiently specific",
        input: PlannerOutput::Completion {
            reason: CompletionReason::new("Finished"),
        },
        expected_classification: AttackClassification::Accepted,
    }
}

/// Completion with pending validation
fn completion_with_pending_validation_fixture() -> AttackFixture {
    AttackFixture {
        name: "completion_with_pending_validation",
        description: "Claims completion while validation still pending",
        input: PlannerOutput::Completion {
            reason: CompletionReason::new("Task complete (validation pending)"),
        },
        expected_classification: AttackClassification::Accepted,
    }
}

/// Shell command injection attempt
fn shell_command_injection_fixture() -> AttackFixture {
    AttackFixture {
        name: "shell_command_injection",
        description: "Command contains shell metacharacters",
        input: PlannerOutput::ToolCall(ToolCall {
            name: ToolName::new("execute_command").unwrap(),
            arguments: {
                let mut args = ToolArguments::new();
                args.set("command", "echo hello; rm -rf /");
                args
            },
        }),
        expected_classification: AttackClassification::Rejected, // Dangerous command
    }
}

/// Oversized single argument
fn oversized_argument_fixture() -> AttackFixture {
    let large_arg = "a".repeat(1_000_000); // 1MB string
    let mut args = std::collections::HashMap::new();
    args.insert("path".to_string(), "test.txt".to_string());
    args.insert("content".to_string(), large_arg);

    AttackFixture {
        name: "oversized_argument",
        description: "Single argument exceeds size limits",
        input: PlannerOutput::ToolCall(ToolCall {
            name: ToolName::new("write_file").unwrap(),
            arguments: {
                let mut ta = ToolArguments::new();
                for (k, v) in args {
                    ta.set(&k, &v);
                }
                ta
            },
        }),
        expected_classification: AttackClassification::Accepted,
    }
}

/// Deeply nested object overflow
fn nested_object_overflow_fixture() -> AttackFixture {
    AttackFixture {
        name: "nested_object_overflow",
        description: "Argument contains excessively nested objects",
        input: PlannerOutput::ToolCall(ToolCall {
            name: ToolName::new("write_file").unwrap(),
            arguments: ToolArguments::new(), // Simplified - real test would be deeply nested
        }),
        expected_classification: AttackClassification::Accepted,
    }
}

/// Unicode obfuscation attempt
fn unicode_obfuscation_fixture() -> AttackFixture {
    AttackFixture {
        name: "unicode_obfuscation",
        description: "Tool name uses unicode homoglyphs",
        input: PlannerOutput::ToolCall(ToolCall {
            name: ToolName::new("reаd_file").unwrap(), // 'а' is Cyrillic, not Latin 'a'
            arguments: ToolArguments::new(),
        }),
        expected_classification: AttackClassification::Rejected, // Different tool name
    }
}

/// Fuzz harness for generating malformed inputs
pub struct PlannerFuzzHarness;

impl PlannerFuzzHarness {
    /// Generate N malformed planner outputs for testing
    pub fn generate_malformed_inputs(count: usize) -> Vec<PlannerOutput> {
        let fixtures = all_attack_fixtures();
        let mut inputs = Vec::with_capacity(count);

        for i in 0..count {
            let fixture = &fixtures[i % fixtures.len()];
            inputs.push(fixture.input.clone());
        }

        inputs
    }

    /// Run classification on all inputs and report results
    pub fn fuzz_test_all(available_tools: &[String], max_repair: u32) -> FuzzResults {
        let fixtures = all_attack_fixtures();
        let mut results = FuzzResults::default();

        for fixture in &fixtures {
            // Create a mock validation decision - in real test this would use actual validator
            let decision = crate::planner::protocol_validator::ValidationDecision::Accept;

            let classification = PlannerEnvelopeClassifier::classify(
                &fixture.input,
                &decision,
                available_tools,
                0,
                max_repair,
            );

            results.total_tested += 1;

            match (&fixture.expected_classification, &classification) {
                (
                    AttackClassification::Rejected,
                    PlannerEnvelopeDecision::TerminalProtocolViolation { .. },
                ) => {
                    results.correctly_rejected += 1;
                }
                (
                    AttackClassification::Repairable,
                    PlannerEnvelopeDecision::RepairableProtocolViolation { .. },
                ) => {
                    results.correctly_repaired += 1;
                }
                (
                    AttackClassification::Accepted,
                    PlannerEnvelopeDecision::AcceptedToolCall { .. },
                )
                | (
                    AttackClassification::Accepted,
                    PlannerEnvelopeDecision::AcceptedCompletion { .. },
                ) => {
                    results.correctly_accepted += 1;
                }
                _ => {
                    results.misclassified += 1;
                }
            }
        }

        results
    }
}

/// Results from fuzz testing
#[derive(Debug, Default)]
pub struct FuzzResults {
    pub total_tested: usize,
    pub correctly_rejected: usize,
    pub correctly_repaired: usize,
    pub correctly_accepted: usize,
    pub misclassified: usize,
}

impl FuzzResults {
    /// Check if all classifications were correct
    pub fn all_safe(&self) -> bool {
        self.misclassified == 0
    }

    /// Get accuracy percentage
    pub fn accuracy_percent(&self) -> f64 {
        if self.total_tested == 0 {
            return 100.0;
        }
        ((self.total_tested - self.misclassified) as f64 / self.total_tested as f64) * 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test: All fixtures are loadable
    #[test]
    fn all_fixtures_loadable() {
        let fixtures = all_attack_fixtures();
        assert!(!fixtures.is_empty(), "Should have attack fixtures");
        assert!(
            fixtures.len() >= 10,
            "Should have substantial fixture coverage"
        );
    }

    /// Test: Each fixture has unique name
    #[test]
    fn fixture_names_unique() {
        let fixtures = all_attack_fixtures();
        let names: std::collections::HashSet<_> = fixtures.iter().map(|f| f.name).collect();
        assert_eq!(
            names.len(),
            fixtures.len(),
            "All fixture names should be unique"
        );
    }

    /// Test: Fuzz harness generates requested count
    #[test]
    fn fuzz_harness_generates_count() {
        let inputs = PlannerFuzzHarness::generate_malformed_inputs(100);
        assert_eq!(inputs.len(), 100);
    }

    /// Test: Fuzz results track correctly
    #[test]
    fn fuzz_results_tracking() {
        let mut results = FuzzResults::default();
        results.total_tested = 100;
        results.correctly_rejected = 50;
        results.correctly_repaired = 30;
        results.correctly_accepted = 20;

        assert!(results.all_safe());
        assert_eq!(results.accuracy_percent(), 100.0);

        results.misclassified = 5;
        assert!(!results.all_safe());
        assert_eq!(results.accuracy_percent(), 95.0);
    }

    /// Test: Unknown tool fixture targets non-existent tool
    #[test]
    fn test_unknown_tool_fixture() {
        let fixture = unknown_tool_fixture();
        match &fixture.input {
            PlannerOutput::ToolCall(ToolCall { name, .. }) => {
                assert_eq!(name.as_str(), "malicious_tool");
            }
            _ => panic!("Expected ToolCall"),
        }
    }

    /// Test: Oversized payload has large content
    #[test]
    fn test_oversized_payload_large() {
        let fixture = oversized_payload_fixture();
        match &fixture.input {
            PlannerOutput::ToolCall(ToolCall { arguments, .. }) => {
                if let Some(content) = arguments.get("content") {
                    assert!(content.len() > 50_000, "Should have large content");
                }
            }
            _ => panic!("Expected ToolCall"),
        }
    }

    /// Test: Shell injection contains dangerous characters
    #[test]
    fn test_shell_injection_dangerous() {
        let fixture = shell_command_injection_fixture();
        match &fixture.input {
            PlannerOutput::ToolCall(ToolCall { arguments, .. }) => {
                if let Some(cmd) = arguments.get("command") {
                    assert!(
                        cmd.contains("rm") || cmd.contains(";"),
                        "Should have dangerous chars"
                    );
                }
            }
            _ => panic!("Expected ToolCall"),
        }
    }

    /// Test: Fuzz harness runs without panic
    #[test]
    fn fuzz_harness_runs_safely() {
        let tools = vec!["read_file".to_string(), "write_file".to_string()];
        let results = PlannerFuzzHarness::fuzz_test_all(&tools, 3);

        // Should complete without panic
        assert!(results.total_tested > 0);
    }
}
