//! Documentation Generation Module (V2.5)
//!
//! Chain-aware multi-document generation that breaks large documentation
//! tasks into manageable steps within Rasputin's bounded execution limits.

use crate::persistence::{ChainLifecycleStatus, ChainStepStatus, PersistentChain, PersistentChainStep};
use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// The 15 canonical documentation definitions
pub const CANONICAL_DOCS: &[DocDefinition] = &[
    DocDefinition {
        number: 1,
        filename: "01_PROJECT_OVERVIEW.md",
        title: "Project Overview",
        purpose: "High-level purpose, goals, value proposition, and elevator pitch",
        sections: &["Elevator Pitch", "Core Purpose", "Quick Start", "Design Philosophy"],
    },
    DocDefinition {
        number: 2,
        filename: "02_ARCHITECTURE.md",
        title: "Architecture",
        purpose: "System architecture, layers, components, data flow",
        sections: &["System Overview", "Truth Layers", "Data Flow", "Components"],
    },
    DocDefinition {
        number: 3,
        filename: "03_TECHNOLOGY_STACK.md",
        title: "Technology Stack",
        purpose: "Languages, frameworks, libraries, tools, versions",
        sections: &["Languages", "Dependencies", "Build Configuration", "Rationale"],
    },
    DocDefinition {
        number: 4,
        filename: "04_CORE_CONCEPTS.md",
        title: "Core Concepts",
        purpose: "Domain model, abstractions, patterns, terminology",
        sections: &["Truth Layers", "Domain Model", "Bounded Execution", "Glossary"],
    },
    DocDefinition {
        number: 5,
        filename: "05_FOLDER_STRUCTURE.md",
        title: "Folder Structure",
        purpose: "Directory layout, purpose of each folder/module",
        sections: &["Repository Layout", "Directory Purposes", "Key Files", "Data Paths"],
    },
    DocDefinition {
        number: 6,
        filename: "06_MAIN_WORKFLOWS.md",
        title: "Main Workflows",
        purpose: "Critical flows and system processes",
        sections: &["Startup Flow", "Chat Flow", "Goal Flow", "Chain Execution"],
    },
    DocDefinition {
        number: 7,
        filename: "07_API_REFERENCE.md",
        title: "API Reference",
        purpose: "Public APIs, endpoints, interfaces with examples",
        sections: &["Commands", "Types", "Tools", "Environment Variables"],
    },
    DocDefinition {
        number: 8,
        filename: "08_DATA_MODEL.md",
        title: "Data Model",
        purpose: "Schemas, entities, relationships, data flows",
        sections: &["PersistentState", "PersistentChain", "AuditLog", "Validation"],
    },
    DocDefinition {
        number: 9,
        filename: "09_CONFIGURATION.md",
        title: "Configuration",
        purpose: "Config options, environment variables, setup, defaults",
        sections: &["Config Files", "Environment Variables", "Ollama Setup", "Defaults"],
    },
    DocDefinition {
        number: 10,
        filename: "10_DEVELOPMENT_GUIDE.md",
        title: "Development Guide",
        purpose: "Setup, build, test, debug, contribute",
        sections: &["Prerequisites", "Building", "Testing", "Debugging"],
    },
    DocDefinition {
        number: 11,
        filename: "11_TESTING_STRATEGY.md",
        title: "Testing Strategy",
        purpose: "Testing approach, coverage, patterns",
        sections: &["Test Organization", "Coverage", "CI/CD", "Patterns"],
    },
    DocDefinition {
        number: 12,
        filename: "12_DEPLOYMENT_AND_OPERATIONS.md",
        title: "Deployment and Operations",
        purpose: "Build, deploy, CI/CD, monitoring, scaling",
        sections: &["Deployment", "Installation", "Operations", "Monitoring"],
    },
    DocDefinition {
        number: 13,
        filename: "13_SECURITY_AND_COMPLIANCE.md",
        title: "Security and Compliance",
        purpose: "Security practices, vulnerabilities, compliance",
        sections: &["Security Architecture", "Controls", "Privacy", "Compliance"],
    },
    DocDefinition {
        number: 14,
        filename: "14_KNOWN_LIMITATIONS_AND_TRADEOFFS.md",
        title: "Known Limitations and Tradeoffs",
        purpose: "Honest assessment of limitations, tradeoffs, debt",
        sections: &["Limitations", "Tradeoffs", "Boundaries", "Invariants"],
    },
    DocDefinition {
        number: 15,
        filename: "15_FUTURE_ROADMAP_AND_EXTENSIBILITY.md",
        title: "Future Roadmap and Extensibility",
        purpose: "Planned improvements, extensibility, contributions",
        sections: &["Roadmap", "Phases", "Extensibility", "Contributions"],
    },
];

/// Definition of a single documentation file
#[derive(Debug, Clone)]
pub struct DocDefinition {
    pub number: u8,
    pub filename: &'static str,
    pub title: &'static str,
    pub purpose: &'static str,
    pub sections: &'static [&'static str],
}

/// State for documentation generation chain
#[derive(Debug, Clone)]
pub struct DocGenerationState {
    pub repo_path: PathBuf,
    pub output_dir: PathBuf,
    pub current_step: usize, // 0-15, 0 = not started
    pub generated_docs: HashMap<u8, DocStatus>,
    pub chain_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DocStatus {
    Pending,
    InProgress,
    Completed { path: PathBuf },
    Failed { error: String },
}

impl DocGenerationState {
    pub fn new(repo_path: PathBuf, output_dir: PathBuf) -> Self {
        let mut generated_docs = HashMap::new();
        for doc in CANONICAL_DOCS {
            generated_docs.insert(doc.number, DocStatus::Pending);
        }

        Self {
            repo_path,
            output_dir,
            current_step: 0,
            generated_docs,
            chain_id: None,
        }
    }

    pub fn progress(&self) -> (usize, usize) {
        let completed = self
            .generated_docs
            .values()
            .filter(|s| matches!(s, DocStatus::Completed { .. }))
            .count();
        (completed, CANONICAL_DOCS.len())
    }

    pub fn is_complete(&self) -> bool {
        self.generated_docs
            .values()
            .all(|s| matches!(s, DocStatus::Completed { .. }))
    }

    pub fn get_next_pending(&self) -> Option<&'static DocDefinition> {
        CANONICAL_DOCS
            .iter()
            .find(|d| matches!(self.generated_docs.get(&d.number), Some(DocStatus::Pending)))
    }

    pub fn get_doc_status(&self, number: u8) -> Option<&DocStatus> {
        self.generated_docs.get(&number)
    }
}

/// Generate a single documentation file
pub async fn generate_single_doc(
    doc: &DocDefinition,
    repo_path: &Path,
    output_dir: &Path,
) -> Result<PathBuf> {
    info!("Generating document {}: {}", doc.number, doc.filename);

    // Create output directory if needed
    tokio::fs::create_dir_all(output_dir).await?;

    let output_path = output_dir.join(doc.filename);

    // Generate content based on repository analysis
    let content = generate_doc_content(doc, repo_path).await?;

    // Write file
    tokio::fs::write(&output_path, content).await?;

    info!("Generated: {}", output_path.display());
    Ok(output_path)
}

/// Generate content for a specific document
async fn generate_doc_content(doc: &DocDefinition, repo_path: &Path) -> Result<String> {
    // In a real implementation, this would:
    // 1. Analyze the repository structure
    // 2. Read relevant source files
    // 3. Use the LLM to generate comprehensive content
    // 4. Apply templates and formatting

    // For now, generate a structured template
    let mut content = String::new();

    // Header
    content.push_str(&format!("# {}\n\n", doc.title));
    content.push_str("## Overview\n\n");
    content.push_str(&format!("{}\n\n", doc.purpose));

    // Sections
    for section in doc.sections {
        content.push_str(&format!("## {}\n\n", section));
        content.push_str(&format!("Content for {} section.\n\n", section));
        content.push_str("- Key point 1\n");
        content.push_str("- Key point 2\n");
        content.push_str("- Key point 3\n\n");
    }

    // Footer
    content.push_str("---\n\n");
    content.push_str("*Canonical documentation generated by Rasputin*\n");

    Ok(content)
}

/// Create a persistent chain for documentation generation
pub fn create_doc_chain(repo_path: &Path, output_dir: &Path) -> PersistentChain {
    let steps: Vec<PersistentChainStep> = CANONICAL_DOCS
        .iter()
        .map(|doc| PersistentChainStep {
            id: format!("doc-{}", doc.number),
            description: format!("Generate {} - {}", doc.filename, doc.title),
            status: ChainStepStatus::Pending,
            retry_of: None,
            retry_attempt: 0,
            execution_outcome: None,
            execution_result_class: None,
            execution_results: vec![],
            failure_reason: None,
            recovery_step_kind: None,
            evidence_snapshot: None,
            force_override_used: false,
            tool_calls: vec![],
            result_summary: None,
            validation_passed: None,
            started_at: None,
            completed_at: None,
            error_message: None,
            replay_record: None,
        })
        .collect();

    PersistentChain {
        id: format!("doc-gen-{}", uuid::Uuid::new_v4()),
        name: "Documentation Generation".to_string(),
        objective: format!(
            "Generate 15 canonical documentation files for {}",
            repo_path.display()
        ),
        raw_prompt: String::new(),
        status: ChainLifecycleStatus::Ready,
        steps,
        active_step: Some(0),
        repo_path: Some(repo_path.to_string_lossy().to_string()),
        conversation_id: None,
        created_at: chrono::Local::now(),
        updated_at: chrono::Local::now(),
        completed_at: None,
        archived: false,
        total_steps_executed: 0,
        total_steps_failed: 0,
        execution_outcome: None,
        force_override_used: false,
        objective_satisfaction: Default::default(),
        selected_context_files: vec![],
        context_state: None,
        pending_checkpoint: None,
        git_grounding: None,
        audit_log: Default::default(),
    }
}

/// Validate generated documentation
pub fn validate_generated_docs(output_dir: &Path) -> Result<ValidationReport> {
    let mut report = ValidationReport {
        total: CANONICAL_DOCS.len(),
        valid: 0,
        invalid: 0,
        errors: vec![],
    };

    for doc in CANONICAL_DOCS {
        let path = output_dir.join(doc.filename);

        if !path.exists() {
            report.invalid += 1;
            report
                .errors
                .push(format!("{}: File not found", doc.filename));
            continue;
        }

        // Read and validate content
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                // Check required elements
                let mut errors = vec![];

                if !content.starts_with(&format!("# {}", doc.title)) {
                    errors.push(format!("Missing or invalid title"));
                }

                for section in doc.sections {
                    if !content.contains(&format!("## {}", section)) {
                        errors.push(format!("Missing section: {}", section));
                    }
                }

                if errors.is_empty() {
                    report.valid += 1;
                } else {
                    report.invalid += 1;
                    report.errors.push(format!("{}: {}", doc.filename, errors.join(", ")));
                }
            }
            Err(e) => {
                report.invalid += 1;
                report
                    .errors
                    .push(format!("{}: Failed to read - {}", doc.filename, e));
            }
        }
    }

    Ok(report)
}

/// Validation report for generated documentation
#[derive(Debug)]
pub struct ValidationReport {
    pub total: usize,
    pub valid: usize,
    pub invalid: usize,
    pub errors: Vec<String>,
}

impl ValidationReport {
    pub fn is_valid(&self) -> bool {
        self.invalid == 0 && self.valid == self.total
    }
}

/// Get documentation generation status as formatted string
pub fn format_status(state: &DocGenerationState) -> String {
    let (completed, total) = state.progress();
    let percentage = (completed as f32 / total as f32) * 100.0;

    let mut status = format!(
        "Documentation Generation Status\n\
         =================================\n\
         Progress: {}/{} ({:.1}%)\n\n",
        completed, total, percentage
    );

    for doc in CANONICAL_DOCS {
        let symbol = match state.generated_docs.get(&doc.number) {
            Some(DocStatus::Completed { .. }) => "✅",
            Some(DocStatus::InProgress) => "⏳",
            Some(DocStatus::Failed { .. }) => "❌",
            _ => "⬜",
        };
        status.push_str(&format!("{} {}\n", symbol, doc.filename));
    }

    status
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canonical_docs_count() {
        assert_eq!(CANONICAL_DOCS.len(), 15);
    }

    #[test]
    fn test_doc_numbering() {
        for (i, doc) in CANONICAL_DOCS.iter().enumerate() {
            assert_eq!(doc.number as usize, i + 1);
        }
    }

    #[test]
    fn test_progress_calculation() {
        let temp_dir = tempfile::tempdir().unwrap();
        let state = DocGenerationState::new(
            PathBuf::from("/repo"),
            temp_dir.path().to_path_buf(),
        );

        let (completed, total) = state.progress();
        assert_eq!(completed, 0);
        assert_eq!(total, 15);
    }
}
