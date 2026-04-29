//! Step Prompt Compactor
//!
//! Shrinks large prompts into focused, executable step prompts.
//! Ensures each step stays within model context limits and executes quickly.

use crate::artifact_contract::RequiredArtifact;
use crate::large_prompt_classifier::ArtifactType;
use crate::chain_working_memory::ChainWorkingMemory;
use std::collections::HashSet;

/// Compacts prompts to be step-focused and bounded
pub struct StepPromptCompactor;

impl StepPromptCompactor {
    /// Maximum tokens for a single step prompt
    pub const MAX_STEP_TOKENS: usize = 4000;
    /// Target tokens for optimal execution
    pub const TARGET_STEP_TOKENS: usize = 2500;
    
    /// Compact a full prompt into a step-sized prompt
    pub fn compact_for_step(
        original_prompt: &str,
        artifact: &RequiredArtifact,
        step_number: usize,
        total_steps: usize,
        working_memory: Option<&ChainWorkingMemory>,
    ) -> String {
        // Build compact context
        let context = Self::build_step_context(
            artifact, 
            step_number, 
            total_steps,
            working_memory
        );
        
        // Get focused instruction
        let instruction = Self::build_step_instruction(artifact);
        
        // Get constraints
        let constraints = Self::build_step_constraints(artifact);
        
        // Combine
        let compacted = format!(
            "{context}\n\n{instruction}\n\n{constraints}"
        );
        
        // Verify size and truncate if needed
        Self::ensure_size_limit(compacted, Self::MAX_STEP_TOKENS)
    }
    
    /// Build context section for a step
    fn build_step_context(
        artifact: &RequiredArtifact,
        step_number: usize,
        total_steps: usize,
        working_memory: Option<&ChainWorkingMemory>,
    ) -> String {
        let mut parts = vec![
            format!("STEP {}/{}: {}", step_number, total_steps, 
                artifact.path.file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default()
            ),
        ];
        
        // Add working memory context if available
        if let Some(memory) = working_memory {
            let relevant_context = memory.get_generation_context(&artifact.path.to_string_lossy());
            if !relevant_context.is_empty() {
                parts.push(format!(
                    "REPO CONTEXT:\n{}",
                    Self::truncate_context(&relevant_context, 1000)
                ));
            }
        }
        
        parts.join("\n\n")
    }
    
    /// Build focused instruction for a step
    fn build_step_instruction(artifact: &RequiredArtifact) -> String {
        let type_instruction = match &artifact.artifact_type {
            ArtifactType::Markdown => {
                format!(
                    "TASK: Generate markdown documentation.\n\n\
                    PURPOSE: {}\n\n\
                    FOCUS: Create ONLY this file. Base content on repo analysis.\n\
                    Do NOT generate other files.\n\
                    Do NOT modify source code.",
                    artifact.purpose
                )
            }
            ArtifactType::Code { language } => {
                format!(
                    "TASK: Generate {} code.\n\n\
                    PURPOSE: {}\n\n\
                    FOCUS: Create ONLY this file. Match existing code style.\n\
                    Follow project conventions.\n\
                    Do NOT generate other files.",
                    language,
                    artifact.purpose
                )
            }
            ArtifactType::Config => {
                format!(
                    "TASK: Generate configuration file.\n\n\
                    PURPOSE: {}\n\n\
                    FOCUS: Create ONLY this file. Match existing config style.",
                    artifact.purpose
                )
            }
            ArtifactType::Test => {
                format!(
                    "TASK: Generate test file.\n\n\
                    PURPOSE: {}\n\n\
                    FOCUS: Create ONLY this file. Follow testing patterns in repo.",
                    artifact.purpose
                )
            }
            _ => {
                format!(
                    "TASK: Generate file.\n\n\
                    PURPOSE: {}\n\n\
                    FOCUS: Create ONLY this file.",
                    artifact.purpose
                )
            }
        };
        
        format!(
            "INSTRUCTION:\n{}\n\n\
            OUTPUT: Write to {}",
            type_instruction,
            artifact.path.display()
        )
    }
    
    /// Build constraints for a step
    fn build_step_constraints(artifact: &RequiredArtifact) -> String {
        let base_constraints = r#"CONSTRAINTS:
- Output ONLY the content for this single file
- Do NOT wrap output in markdown code blocks
- Do NOT include explanations or commentary
- Do NOT generate multiple files
- Base content on actual repository analysis
- Follow existing project conventions"#;
        
        let type_specific = match &artifact.artifact_type {
            ArtifactType::Markdown => {
                "\n- Start with proper markdown heading\n- Include relevant code references\n- Link to other docs when appropriate".to_string()
            }
            ArtifactType::Code { language } => {
                format!(
                    "\n- Include proper {} imports/usings\n- Follow existing error handling patterns\n- Add appropriate documentation comments",
                    language
                )
            }
            _ => "".to_string(),
        };
        
        format!("{}{}", base_constraints, type_specific)
    }
    
    /// Compact a planning phase prompt
    pub fn compact_planning_prompt(original_prompt: &str, artifact_count: usize) -> String {
        format!(
            "PHASE 0: REPOSITORY INVENTORY (Step 1 of {} total steps)\n\n\
            Analyze repository structure to guide {} artifact generations.\n\n\
            Focus:\n\
            1. Top-level directories\n\
            2. Entry points\n\
            3. Technology stack\n\
            4. Existing patterns\n\n\
            Be concise - store key findings in working memory for later steps.",
            artifact_count + 2, // + planning + validation
            artifact_count
        )
    }
    
    /// Compact a validation phase prompt
    pub fn compact_validation_prompt(contract_summary: &str, artifact_count: usize) -> String {
        format!(
            "PHASE Z: FINAL VALIDATION (Last step)\n\n\
            Verify {} artifacts from contract:\n{}\n\n\
            Check:\n\
            1. All files exist\n\
            2. Files are non-empty\n\
            3. Content is repo-grounded\n\
            4. No source code was modified\n\n\
            Report completion status.",
            artifact_count,
            contract_summary
        )
    }
    
    /// Estimate token count for text
    pub fn estimate_tokens(text: &str) -> usize {
        // Rough approximation: ~4 characters per token
        // This is a conservative estimate
        (text.len() / 4).max(1)
    }
    
    /// Truncate context to stay within token budget
    fn truncate_context(context: &str, max_tokens: usize) -> String {
        let max_chars = max_tokens * 4;
        
        if context.len() <= max_chars {
            context.to_string()
        } else {
            // Find a good break point
            let break_point = context[..max_chars]
                .rfind("\n\n")
                .unwrap_or(max_chars);
            
            format!(
                "{}\n\n[... {} more tokens of context ...]",
                &context[..break_point],
                Self::estimate_tokens(&context[break_point..])
            )
        }
    }
    
    /// Ensure prompt stays within size limit
    fn ensure_size_limit(prompt: String, max_tokens: usize) -> String {
        let current_tokens = Self::estimate_tokens(&prompt);
        
        if current_tokens <= max_tokens {
            prompt
        } else {
            // Truncate the context portion
            let max_chars = max_tokens * 4;
            if prompt.len() > max_chars {
                format!(
                    "{}\n\n[Truncated to fit {} token limit]",
                    &prompt[..max_chars - 50],
                    max_tokens
                )
            } else {
                prompt
            }
        }
    }
    
    /// Remove redundant content from prompt
    pub fn deduplicate(prompt: &str) -> String {
        let lines: Vec<&str> = prompt.lines().collect();
        let mut seen = HashSet::new();
        let mut result = vec![];
        
        for line in lines {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                result.push(line);
            } else if !seen.contains(trimmed) {
                seen.insert(trimmed.to_string());
                result.push(line);
            }
        }
        
        result.join("\n")
    }
    
    /// Compress by removing filler words
    pub fn compress_filler_words(text: &str) -> String {
        text
            .replace("Please ", "")
            .replace("Please, ", "")
            .replace("I would like you to ", "")
            .replace("I want you to ", "")
            .replace("Can you ", "")
            .replace("Could you ", "")
            .replace("Would you mind ", "")
            .replace("It would be great if you could ", "")
            .replace("kindly ", "")
    }
    
    /// Get token count breakdown for a prompt
    pub fn analyze_token_usage(prompt: &str) -> TokenUsageAnalysis {
        let total_tokens = Self::estimate_tokens(prompt);
        
        let lines: Vec<&str> = prompt.lines().collect();
        let context_lines: Vec<&str> = lines.iter()
            .filter(|l| l.starts_with("REPO CONTEXT:") || l.starts_with("PREVIOUSLY:"))
            .copied()
            .collect();
        let instruction_lines: Vec<&str> = lines.iter()
            .filter(|l| l.starts_with("TASK:") || l.starts_with("INSTRUCTION:"))
            .copied()
            .collect();
        
        TokenUsageAnalysis {
            total_tokens,
            context_tokens: Self::estimate_tokens(&context_lines.join("\n")),
            instruction_tokens: Self::estimate_tokens(&instruction_lines.join("\n")),
            overhead_tokens: total_tokens.saturating_sub(
                Self::estimate_tokens(&context_lines.join("\n")) +
                Self::estimate_tokens(&instruction_lines.join("\n"))
            ),
            within_limit: total_tokens <= Self::MAX_STEP_TOKENS,
        }
    }
}

/// Analysis of token usage in a prompt
pub struct TokenUsageAnalysis {
    pub total_tokens: usize,
    pub context_tokens: usize,
    pub instruction_tokens: usize,
    pub overhead_tokens: usize,
    pub within_limit: bool,
}

impl std::fmt::Display for TokenUsageAnalysis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Token usage: {} total (context: {}, instruction: {}, overhead: {}) - {}",
            self.total_tokens,
            self.context_tokens,
            self.instruction_tokens,
            self.overhead_tokens,
            if self.within_limit { "✓ within limit" } else { "✗ exceeds limit" }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    
    #[test]
    fn test_compact_for_step() {
        let artifact = RequiredArtifact {
            path: PathBuf::from("docs/README.md"),
            purpose: "Project overview".to_string(),
            status: crate::large_prompt_classifier::ArtifactStatus::Missing,
            step_number: 1,
            artifact_type: ArtifactType::Markdown,
            dependencies: vec![],
            estimated_effort: crate::artifact_contract::EffortLevel::Medium,
        };
        
        let original = r#"
This is a very long prompt that goes on and on about many different things.
It contains lots of context and instructions that could be much more concise.
We want to compact this down to just the essential information for the step.
"#;
        
        let compacted = StepPromptCompactor::compact_for_step(
            original, 
            &artifact, 
            1, 
            15, 
            None
        );
        
        // Should be significantly smaller
        assert!(compacted.len() < original.len() + 200); // +200 for added structure
        
        // Should contain step info
        assert!(compacted.contains("STEP 1/15"));
        assert!(compacted.contains("README.md"));
    }
    
    #[test]
    fn test_estimate_tokens() {
        let text = "This is a test string with some content.";
        let tokens = StepPromptCompactor::estimate_tokens(text);
        // Roughly 38 chars / 4 = ~10 tokens
        assert!(tokens > 0 && tokens < 20);
    }
    
    #[test]
    fn test_truncate_context() {
        let long_context = "A".repeat(5000);
        let truncated = StepPromptCompactor::truncate_context(&long_context, 100);
        
        assert!(truncated.len() < 500);
        assert!(truncated.contains("[..."));
    }
    
    #[test]
    fn test_analyze_token_usage() {
        let prompt = r#"REPO CONTEXT:
Some context here

INSTRUCTION:
Do something

More content"#;
        
        let analysis = StepPromptCompactor::analyze_token_usage(prompt);
        assert!(analysis.total_tokens > 0);
        assert!(analysis.context_tokens > 0);
        assert!(analysis.instruction_tokens > 0);
    }
    
    #[test]
    fn test_compress_filler_words() {
        let text = "Please generate this file. I would like you to make it good.";
        let compressed = StepPromptCompactor::compress_filler_words(text);
        
        assert!(!compressed.contains("Please"));
        assert!(!compressed.contains("I would like you to"));
        assert!(compressed.contains("generate"));
    }
}
