//! Chain Working Memory
//!
//! Maintains context across chain steps so each step builds on previous work.
//! Prevents redundant repository analysis and enables progressive refinement.

use crate::persistence::PersistentChain;
use std::collections::HashMap;
use std::path::PathBuf;
use serde::{Serialize, Deserialize};
use tracing::{info, debug};

/// Working memory for a chain execution
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChainWorkingMemory {
    /// Chain this memory belongs to
    pub chain_id: String,
    /// Repository structure learned in Phase 0
    pub repo_structure: Option<RepoStructure>,
    /// Source code analysis from Phase 1
    pub source_analysis: Option<SourceAnalysis>,
    /// Completed artifacts with their summaries
    pub completed_artifacts: HashMap<String, ArtifactSummary>,
    /// Key findings across all phases
    pub key_findings: Vec<String>,
    /// Dependencies discovered
    pub discovered_dependencies: Vec<String>,
    /// Architecture patterns identified
    pub architecture_patterns: Vec<String>,
    /// Public APIs and interfaces
    pub public_apis: Vec<ApiDefinition>,
    /// Configuration points
    pub config_points: Vec<ConfigPoint>,
    /// Data flow patterns
    pub data_flows: Vec<DataFlow>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoStructure {
    pub root_path: PathBuf,
    pub top_level_dirs: Vec<DirectoryInfo>,
    pub key_files: Vec<FileInfo>,
    pub entry_points: Vec<String>,
    pub tech_stack: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectoryInfo {
    pub name: String,
    pub purpose: String,
    pub file_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub path: String,
    pub purpose: String,
    pub lines_of_code: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceAnalysis {
    pub modules: Vec<ModuleInfo>,
    pub dependencies: Vec<DependencyInfo>,
    pub complexity_score: u8, // 0-100
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleInfo {
    pub name: String,
    pub path: String,
    pub exports: Vec<String>,
    pub imports: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyInfo {
    pub name: String,
    pub version: String,
    pub purpose: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactSummary {
    pub path: String,
    pub content_summary: String,
    pub key_sections: Vec<String>,
    pub references_source_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiDefinition {
    pub name: String,
    pub signature: String,
    pub location: String,
    pub purpose: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigPoint {
    pub key: String,
    pub default_value: String,
    pub location: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataFlow {
    pub from: String,
    pub to: String,
    pub data_type: String,
    pub pattern: String,
}

impl ChainWorkingMemory {
    /// Create new working memory for a chain
    pub fn new(chain_id: impl Into<String>) -> Self {
        Self {
            chain_id: chain_id.into(),
            repo_structure: None,
            source_analysis: None,
            completed_artifacts: HashMap::new(),
            key_findings: vec![],
            discovered_dependencies: vec![],
            architecture_patterns: vec![],
            public_apis: vec![],
            config_points: vec![],
            data_flows: vec![],
        }
    }
    
    /// Store repository structure from Phase 0
    pub fn set_repo_structure(&mut self, structure: RepoStructure) {
        info!("Working memory: Stored repo structure with {} top-level dirs", 
            structure.top_level_dirs.len());
        self.repo_structure = Some(structure);
    }
    
    /// Store source analysis from Phase 1
    pub fn set_source_analysis(&mut self, analysis: SourceAnalysis) {
        info!("Working memory: Stored source analysis with {} modules", 
            analysis.modules.len());
        self.source_analysis = Some(analysis);
    }
    
    /// Add completed artifact summary
    pub fn add_completed_artifact(&mut self, path: impl Into<String>, summary: ArtifactSummary) {
        let path_str = path.into();
        debug!("Working memory: Added artifact {}", path_str);
        self.completed_artifacts.insert(path_str, summary);
    }
    
    /// Add key finding
    pub fn add_finding(&mut self, finding: impl Into<String>) {
        self.key_findings.push(finding.into());
    }
    
    /// Get context for artifact generation step
    pub fn get_generation_context(&self, target_artifact_path: &str) -> String {
        let mut context_parts = vec![];
        
        // Repo structure context
        if let Some(structure) = &self.repo_structure {
            context_parts.push(format!(
                "REPO STRUCTURE:\n- Root: {}\n- Top dirs: {}\n- Tech stack: {}",
                structure.root_path.display(),
                structure.top_level_dirs.iter().map(|d| d.name.as_str()).collect::<Vec<_>>().join(", "),
                structure.tech_stack.join(", ")
            ));
        }
        
        // Previous artifacts context (to avoid duplication)
        if !self.completed_artifacts.is_empty() {
            let artifacts_summary: Vec<String> = self.completed_artifacts
                .values()
                .map(|a| format!("- {}: {}", a.path, a.content_summary.chars().take(100).collect::<String>()))
                .collect();
            
            context_parts.push(format!(
                "PREVIOUSLY GENERATED (for reference, don't duplicate):\n{}",
                artifacts_summary.join("\n")
            ));
        }
        
        // Key findings
        if !self.key_findings.is_empty() {
            context_parts.push(format!(
                "KEY FINDINGS:\n{}",
                self.key_findings.iter().map(|f| format!("- {}", f)).collect::<Vec<_>>().join("\n")
            ));
        }
        
        context_parts.join("\n\n")
    }
    
    /// Get prompt for planning phase
    pub fn get_planning_prompt(&self) -> String {
        r#"PHASE 0: REPOSITORY INVENTORY

Analyze the repository structure and identify:

1. Top-level directories and their purposes
2. Key source files and entry points  
3. Technology stack and dependencies
4. Existing documentation
5. Project architecture patterns

Store findings in working memory for use in subsequent phases.

Output format:
- Directories: [list with purposes]
- Entry points: [main files]
- Tech stack: [languages, frameworks]
- Patterns: [architectural patterns]
"#.to_string()
    }
    
    /// Get prompt for source mapping phase
    pub fn get_source_mapping_prompt(&self) -> String {
        let base = r#"PHASE 1: SOURCE MAP

Build detailed understanding:

1. Module dependencies and relationships
2. Public APIs and interfaces
3. Data flow patterns
4. Configuration points
5. Complex areas needing attention

Store in working memory for artifact generation reference.

Output format:
- Modules: [list with dependencies]
- APIs: [public interfaces]
- Data flows: [patterns]
- Configs: [configuration points]
"#;
        
        // If we have repo structure, include it
        if let Some(structure) = &self.repo_structure {
            format!(
                "{}\n\nREPO CONTEXT FROM PHASE 0:\n- Top dirs: {}\n- Tech: {}\n",
                base,
                structure.top_level_dirs.iter().map(|d| d.name.as_str()).collect::<Vec<_>>().join(", "),
                structure.tech_stack.join(", ")
            )
        } else {
            base.to_string()
        }
    }
    
    /// Check if we have enough context to proceed
    pub fn has_planning_context(&self) -> bool {
        self.repo_structure.is_some()
    }
    
    /// Get summary of memory state
    pub fn get_summary(&self) -> String {
        let mut parts = vec![
            format!("Working Memory for {}", self.chain_id),
            format!("  Repo structure: {}", if self.repo_structure.is_some() { "✓" } else { "✗" }),
            format!("  Source analysis: {}", if self.source_analysis.is_some() { "✓" } else { "✗" }),
            format!("  Artifacts completed: {}", self.completed_artifacts.len()),
            format!("  Key findings: {}", self.key_findings.len()),
        ];
        
        if !self.architecture_patterns.is_empty() {
            parts.push(format!("  Patterns: {}", self.architecture_patterns.join(", ")));
        }
        
        parts.join("\n")
    }
}

/// Registry of working memories for all chains
pub struct ChainWorkingMemoryRegistry {
    memories: HashMap<String, ChainWorkingMemory>,
}

impl ChainWorkingMemoryRegistry {
    pub fn new() -> Self {
        Self {
            memories: HashMap::new(),
        }
    }
    
    /// Get or create working memory for a chain
    pub fn get_or_create(&mut self, chain_id: impl Into<String>) -> &mut ChainWorkingMemory {
        let id = chain_id.into();
        self.memories.entry(id.clone()).or_insert_with(|| {
            info!("Created working memory for chain: {}", id);
            ChainWorkingMemory::new(id)
        })
    }
    
    /// Get existing memory
    pub fn get(&self, chain_id: &str) -> Option<&ChainWorkingMemory> {
        self.memories.get(chain_id)
    }
    
    /// Remove memory when chain is done
    pub fn remove(&mut self, chain_id: &str) {
        self.memories.remove(chain_id);
    }
    
    /// Clear all memories
    pub fn clear(&mut self) {
        self.memories.clear();
    }
}

impl Default for ChainWorkingMemoryRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_working_memory() {
        let mut memory = ChainWorkingMemory::new("test-chain");
        
        // Set repo structure
        memory.set_repo_structure(RepoStructure {
            root_path: PathBuf::from("/tmp/test"),
            top_level_dirs: vec![
                DirectoryInfo { name: "src".to_string(), purpose: "Source code".to_string(), file_count: 10 },
                DirectoryInfo { name: "tests".to_string(), purpose: "Tests".to_string(), file_count: 5 },
            ],
            key_files: vec![
                FileInfo { path: "src/main.rs".to_string(), purpose: "Entry point".to_string(), lines_of_code: 100 },
            ],
            entry_points: vec!["src/main.rs".to_string()],
            tech_stack: vec!["Rust".to_string(), "Tokio".to_string()],
        });
        
        // Add artifact
        memory.add_completed_artifact("docs/README.md", ArtifactSummary {
            path: "docs/README.md".to_string(),
            content_summary: "Project overview".to_string(),
            key_sections: vec!["Introduction".to_string(), "Setup".to_string()],
            references_source_files: vec!["src/main.rs".to_string()],
        });
        
        // Get generation context
        let context = memory.get_generation_context("docs/ARCHITECTURE.md");
        assert!(context.contains("REPO STRUCTURE"));
        assert!(context.contains("PREVIOUSLY GENERATED"));
        
        // Check summary
        let summary = memory.get_summary();
        assert!(summary.contains("Repo structure: ✓"));
    }
    
    #[test]
    fn test_registry() {
        let mut registry = ChainWorkingMemoryRegistry::new();
        
        // Get or create
        let mem1 = registry.get_or_create("chain-1");
        mem1.add_finding("Important finding");
        
        // Retrieve
        let mem2 = registry.get("chain-1");
        assert!(mem2.is_some());
        assert_eq!(mem2.unwrap().key_findings.len(), 1);
        
        // Remove
        registry.remove("chain-1");
        assert!(registry.get("chain-1").is_none());
    }
}
