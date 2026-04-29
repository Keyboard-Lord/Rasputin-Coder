//! Core types for the Deep Forge air-gapped refinement engine

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;

/// Errors that can occur during the forging process
#[derive(Error, Debug)]
pub enum ForgeError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("Git error: {0}")]
    Git(String),
    
    #[error("Linter error: {0}")]
    Linter(String),
    
    #[error("Ollama API error: {0}")]
    Ollama(String),
    
    #[error("Patch application failed: {0}")]
    PatchFailed(String),
    
    #[error("Test failure: {0}")]
    TestFailed(String),
    
    #[error("Exhaustion loop stalled after {0} iterations")]
    LoopStalled(usize),
    
    #[error("AST parsing error: {0}")]
    AstParse(String),
    
    #[error("Invalid SEARCH/REPLACE block: {0}")]
    InvalidDiff(String),
}

/// A single code flaw identified by the Local Critic
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Flaw {
    /// Unique identifier for this flaw
    pub id: String,
    /// File path relative to repo root
    pub file_path: PathBuf,
    /// Line number (1-indexed)
    pub line: usize,
    /// Priority score (0-100, higher = more critical)
    pub priority: u8,
    /// Category of flaw
    pub category: FlawCategory,
    /// Description of the issue
    pub description: String,
    /// Suggested fix (if available from linter)
    pub suggestion: Option<String>,
    /// Code snippet context
    pub context: String,
    /// Hash of the file content when flaw was detected
    pub content_hash: String,
}

/// Categories of code flaws
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum FlawCategory {
    /// Compiler/linter errors
    Error,
    /// Performance issues
    Performance,
    /// Code style violations
    Style,
    /// Potential bugs
    BugRisk,
    /// Documentation issues
    Documentation,
    /// Security vulnerabilities
    Security,
    /// Test coverage gaps
    TestGap,
    /// Complexity violations
    Complexity,
}

impl FlawCategory {
    /// Get base priority for category
    pub fn base_priority(&self) -> u8 {
        match self {
            FlawCategory::Error => 95,
            FlawCategory::Security => 90,
            FlawCategory::BugRisk => 85,
            FlawCategory::Performance => 75,
            FlawCategory::TestGap => 60,
            FlawCategory::Complexity => 50,
            FlawCategory::Style => 30,
            FlawCategory::Documentation => 20,
        }
    }
}

impl std::fmt::Display for FlawCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            FlawCategory::Error => "error",
            FlawCategory::Security => "security",
            FlawCategory::BugRisk => "bug-risk",
            FlawCategory::Performance => "performance",
            FlawCategory::TestGap => "test-gap",
            FlawCategory::Complexity => "complexity",
            FlawCategory::Style => "style",
            FlawCategory::Documentation => "documentation",
        };
        write!(f, "{}", name)
    }
}

/// The prioritized queue of flaws to fix
#[derive(Debug, Clone, Default)]
pub struct FlawQueue {
    pub flaws: Vec<Flaw>,
    pub processed_count: usize,
    pub max_retries: u8,
}

impl FlawQueue {
    /// Create new empty queue
    pub fn new() -> Self {
        Self {
            flaws: Vec::new(),
            processed_count: 0,
            max_retries: 3,
        }
    }
    
    /// Check if queue is empty
    pub fn is_empty(&self) -> bool {
        self.flaws.is_empty()
    }
    
    /// Get count of remaining flaws
    pub fn len(&self) -> usize {
        self.flaws.len()
    }
    
    /// Pop the highest priority flaw
    pub fn pop_next(&mut self) -> Option<Flaw> {
        if self.flaws.is_empty() {
            return None;
        }
        
        // Find highest priority flaw
        let max_idx = self.flaws.iter()
            .enumerate()
            .max_by_key(|(_, f)| f.priority)
            .map(|(i, _)| i)?;
        
        self.processed_count += 1;
        Some(self.flaws.remove(max_idx))
    }
    
    /// Add a flaw to the queue
    pub fn push(&mut self, flaw: Flaw) {
        self.flaws.push(flaw);
    }
    
    /// Move flaw to back of queue (for retry after failure)
    pub fn retry_later(&mut self, flaw: Flaw) {
        let mut flaw = flaw;
        // Reduce priority on retry
        if flaw.priority > 10 {
            flaw.priority -= 10;
        }
        self.flaws.push(flaw);
    }
    
    /// Sort by priority (descending)
    pub fn sort_by_priority(&mut self) {
        self.flaws.sort_by(|a, b| b.priority.cmp(&a.priority));
    }
}

/// A SEARCH/REPLACE patch block
#[derive(Debug, Clone)]
pub struct Patch {
    pub file_path: PathBuf,
    pub search: String,
    pub replace: String,
    pub description: String,
}

/// Result of applying a patch
#[derive(Debug)]
pub enum PatchResult {
    Success { file_path: PathBuf, applied_at: usize },
    Failed { reason: String },
}

/// Status of the exhaustion loop
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopStatus {
    Running,
    Exhausted,
    Stalled,
    Failed,
}

/// Configuration for the Deep Forge
#[derive(Debug, Clone)]
pub struct ForgeConfig {
    /// Target repository path
    pub target_repo: PathBuf,
    /// Ollama API endpoint
    pub ollama_endpoint: String,
    /// Model to use
    pub model: String,
    /// Timeout for Ollama requests (seconds)
    pub ollama_timeout: u64,
    /// Maximum iterations before considering stalled
    pub max_iterations: usize,
    /// Enable auto-commit
    pub auto_commit: bool,
    /// Linter commands to run
    pub linters: Vec<String>,
    /// Test command
    pub test_command: String,
}

impl Default for ForgeConfig {
    fn default() -> Self {
        Self {
            target_repo: PathBuf::from("."),
            ollama_endpoint: "http://localhost:11434/api/generate".to_string(),
            model: "qwen2.5-coder:14b".to_string(),
            ollama_timeout: 300,
            max_iterations: 100,
            auto_commit: true,
            linters: vec![
                "cargo clippy --all-targets --all-features -- -D warnings".to_string(),
                "cargo audit".to_string(),
            ],
            test_command: "cargo test".to_string(),
        }
    }
}

/// Statistics for the forging session
#[derive(Debug, Clone, Default)]
pub struct ForgeStats {
    pub iterations: usize,
    pub flaws_detected: usize,
    pub flaws_fixed: usize,
    pub patches_applied: usize,
    pub patches_failed: usize,
    pub tests_passed: usize,
    pub tests_failed: usize,
    pub start_time: Option<chrono::DateTime<chrono::Utc>>,
    pub end_time: Option<chrono::DateTime<chrono::Utc>>,
}

/// A code entity discovered by AST parsing
#[derive(Debug, Clone)]
pub struct CodeEntity {
    pub name: String,
    pub kind: EntityKind,
    pub file_path: PathBuf,
    pub start_line: usize,
    pub end_line: usize,
    pub content: String,
    pub complexity_score: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityKind {
    Function,
    Struct,
    Enum,
    Trait,
    Impl,
    Module,
    Macro,
    Test,
}

/// Linter warning/output
#[derive(Debug, Clone)]
pub struct LinterOutput {
    pub tool: String,
    pub file_path: Option<PathBuf>,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub severity: Severity,
    pub message: String,
    pub code: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Info,
}
