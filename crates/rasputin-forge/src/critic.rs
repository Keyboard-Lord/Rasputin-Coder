//! The Local Critic: Air-gapped AST parsing and linting engine

use crate::types::{CodeEntity, EntityKind, Flaw, FlawCategory, FlawQueue, ForgeConfig, ForgeError, LinterOutput, Severity};
use sha2::{Sha256, Digest};

/// Helper function to compute SHA256 hash of content
fn compute_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use tokio::process::Command as TokioCommand;
use tracing::{debug, error, info, warn};
use tree_sitter::{Language, Node, Parser, Query, QueryCursor};
use walkdir::WalkDir;

/// The Local Critic analyzes code and produces a FlawQueue
pub struct LocalCritic {
    config: ForgeConfig,
    parser: Parser,
}

impl LocalCritic {
    /// Create a new Local Critic
    pub fn new(config: ForgeConfig) -> Result<Self, ForgeError> {
        let mut parser = Parser::new();
        
        // Set language based on repo detection
        // For now, assume Rust
        parser.set_language(tree_sitter_rust::language())
            .map_err(|e| ForgeError::AstParse(format!("Failed to set language: {:?}", e)))?;
        
        Ok(Self { config, parser })
    }
    
    /// Run complete audit and generate FlawQueue
    pub async fn audit(&mut self) -> Result<FlawQueue, ForgeError> {
        info!("[CRITIC] Beginning air-gapped audit...");
        
        let mut queue = FlawQueue::new();
        
        // Phase 1: AST-based code analysis
        info!("[CRITIC] Phase 1: AST parsing...");
        let entities = self.parse_repository().await?;
        let ast_flaws = self.analyze_entities(&entities).await?;
        for flaw in ast_flaws {
            queue.push(flaw);
        }
        
        // Phase 2: Local linter execution
        info!("[CRITIC] Phase 2: Local linter pipeline...");
        let linter_flaws = self.run_linters().await?;
        for flaw in linter_flaws {
            queue.push(flaw);
        }
        
        // Phase 3: Test coverage analysis
        info!("[CRITIC] Phase 3: Coverage analysis...");
        let coverage_flaws = self.analyze_test_coverage(&entities).await?;
        for flaw in coverage_flaws {
            queue.push(flaw);
        }
        
        // Sort by priority
        queue.sort_by_priority();
        
        info!("[CRITIC] Audit complete. {} flaws detected.", queue.len());
        
        Ok(queue)
    }
    
    /// Parse all source files in the repository
    async fn parse_repository(&mut self) -> Result<Vec<CodeEntity>, ForgeError> {
        let mut entities = Vec::new();
        
        let source_files = self.discover_source_files().await?;
        
        for file_path in source_files {
            debug!("[CRITIC] Parsing: {:?}", file_path);
            
            let content = tokio::fs::read_to_string(&file_path).await
                .map_err(|e| ForgeError::Io(e))?;
            
            let tree = self.parser.parse(&content, None)
                .ok_or_else(|| ForgeError::AstParse(format!("Failed to parse {:?}", file_path)))?;
            
            let root = tree.root_node();
            let file_entities = self.extract_entities(&root, &file_path, &content)?;
            
            entities.extend(file_entities);
        }
        
        info!("[CRITIC] Discovered {} code entities", entities.len());
        Ok(entities)
    }
    
    /// Discover source files in the repository
    async fn discover_source_files(&self) -> Result<Vec<PathBuf>, ForgeError> {
        let mut files = Vec::new();
        let src_path = self.config.target_repo.join("src");
        
        if !src_path.exists() {
            return Ok(files);
        }
        
        for entry in WalkDir::new(&src_path)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "rs") {
                if !path.to_string_lossy().contains("target/") {
                    files.push(path.to_path_buf());
                }
            }
        }
        
        Ok(files)
    }
    
    /// Extract code entities from AST
    fn extract_entities(
        &self,
        node: &Node,
        file_path: &Path,
        content: &str,
    ) -> Result<Vec<CodeEntity>, ForgeError> {
        let mut entities = Vec::new();
        
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let kind = child.kind();
            
            let entity_kind = match kind {
                "function_item" => Some(EntityKind::Function),
                "struct_item" => Some(EntityKind::Struct),
                "enum_item" => Some(EntityKind::Enum),
                "trait_item" => Some(EntityKind::Trait),
                "impl_item" => Some(EntityKind::Impl),
                "mod_item" => Some(EntityKind::Module),
                "macro_definition" => Some(EntityKind::Macro),
                _ => None,
            };
            
            if let Some(kind) = entity_kind {
                let name = self.extract_name(&child, content);
                let (start_line, end_line) = (child.start_position().row + 1, child.end_position().row + 1);
                let entity_content = content[child.byte_range()].to_string();
                
                // Calculate complexity score
                let complexity = self.calculate_complexity(&child);
                
                entities.push(CodeEntity {
                    name,
                    kind,
                    file_path: file_path.to_path_buf(),
                    start_line,
                    end_line,
                    content: entity_content,
                    complexity_score: complexity,
                });
            }
            
            // Recurse
            let child_entities = self.extract_entities(&child, file_path, content)?;
            entities.extend(child_entities);
        }
        
        Ok(entities)
    }
    
    /// Extract name from node
    fn extract_name(&self, node: &Node, content: &str) -> String {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "identifier" || child.kind() == "type_identifier" {
                return content[child.byte_range()].to_string();
            }
        }
        "unknown".to_string()
    }
    
    /// Calculate complexity score for an entity
    fn calculate_complexity(&self, node: &Node) -> u8 {
        let mut score = 0u8;
        
        // Count control flow constructs
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "if_expression" | "if_let_expression" => score += 2,
                "match_expression" => score += 3,
                "for_expression" | "while_expression" | "loop_expression" => score += 3,
                "match_arm" => score += 1,
                _ => {}
            }
        }
        
        score.min(100)
    }
    
    /// Analyze entities for flaws
    async fn analyze_entities(&self, entities: &[CodeEntity]) -> Result<Vec<Flaw>, ForgeError> {
        let mut flaws = Vec::new();
        
        for entity in entities {
            // Check complexity
            if entity.complexity_score > 15 {
                let flaw = self.create_flaw(
                    entity,
                    FlawCategory::Complexity,
                    format!("High cyclomatic complexity: {}", entity.complexity_score),
                    format!("Function '{}' has complexity score of {}. Consider breaking it into smaller functions.", 
                        entity.name, entity.complexity_score),
                ).await?;
                flaws.push(flaw);
            }
            
            // Check function length
            let lines = entity.end_line - entity.start_line;
            if lines > 50 {
                let flaw = self.create_flaw(
                    entity,
                    FlawCategory::Style,
                    format!("Function too long: {} lines", lines),
                    format!("Function '{}' spans {} lines. Consider extracting helper functions.", 
                        entity.name, lines),
                ).await?;
                flaws.push(flaw);
            }
            
            // Check for TODO/FIXME comments (documentation issue)
            if entity.content.contains("TODO") || entity.content.contains("FIXME") {
                let flaw = self.create_flaw(
                    entity,
                    FlawCategory::Documentation,
                    "Unresolved TODO/FIXME comment".to_string(),
                    format!("Entity '{}' contains unresolved TODO/FIXME comments.", entity.name),
                ).await?;
                flaws.push(flaw);
            }
        }
        
        Ok(flaws)
    }
    
    /// Create a flaw from a code entity
    async fn create_flaw(
        &self,
        entity: &CodeEntity,
        category: FlawCategory,
        description: String,
        suggestion: String,
    ) -> Result<Flaw, ForgeError> {
        let content = tokio::fs::read_to_string(&entity.file_path).await
            .map_err(|e| ForgeError::Io(e))?;
        
        let hash = compute_hash(&content);
        
        let flaw_id = format!("{}-{}-{}-{}", 
            category.base_priority(),
            entity.file_path.file_name().unwrap_or_default().to_string_lossy(),
            entity.start_line,
            &hash[..8.min(hash.len())]
        );
        
        Ok(Flaw {
            id: flaw_id,
            file_path: entity.file_path.strip_prefix(&self.config.target_repo)
                .unwrap_or(&entity.file_path)
                .to_path_buf(),
            line: entity.start_line,
            priority: category.base_priority(),
            category,
            description,
            suggestion: Some(suggestion),
            context: entity.content.lines().take(3).collect::<Vec<_>>().join("\n"),
            content_hash: hash,
        })
    }
    
    /// Run local linters and collect output
    async fn run_linters(&self) -> Result<Vec<Flaw>, ForgeError> {
        let mut all_flaws = Vec::new();
        
        for linter_cmd in &self.config.linters {
            info!("[CRITIC] Running: {}", linter_cmd);
            
            let parts: Vec<&str> = linter_cmd.split_whitespace().collect();
            if parts.is_empty() {
                continue;
            }
            
            let mut cmd = TokioCommand::new(parts[0]);
            cmd.args(&parts[1..])
                .current_dir(&self.config.target_repo)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            
            let output = cmd.output().await.map_err(|e| ForgeError::Io(e))?;
            
            // Parse linter output
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            
            let linter_flaws = self.parse_linter_output(&format!("{}{}", stdout, stderr), linter_cmd).await?;
            all_flaws.extend(linter_flaws);
        }
        
        Ok(all_flaws)
    }
    
    /// Parse linter output into flaws
    async fn parse_linter_output(&self, output: &str, tool: &str) -> Result<Vec<Flaw>, ForgeError> {
        let mut flaws = Vec::new();
        
        // Parse cargo clippy output
        if tool.contains("clippy") {
            for line in output.lines() {
                // Parse clippy warning format: file:line:col: severity: message
                if let Some(flaw) = self.parse_clippy_line(line).await? {
                    flaws.push(flaw);
                }
            }
        }
        
        Ok(flaws)
    }
    
    /// Parse a single clippy line
    async fn parse_clippy_line(&self, line: &str) -> Result<Option<Flaw>, ForgeError> {
        // Simple regex-like parsing for: file.rs:123:45: warning: message
        let parts: Vec<&str> = line.splitn(5, ':').collect();
        if parts.len() < 4 {
            return Ok(None);
        }
        
        let file_path = PathBuf::from(parts[0]);
        let line_num: usize = parts[1].parse().unwrap_or(1);
        let severity = parts[3].trim();
        
        let (category, priority) = match severity {
            "error" => (FlawCategory::Error, 95),
            "warning" => (FlawCategory::BugRisk, 70),
            _ => (FlawCategory::Style, 40),
        };
        
        let message = parts.get(4).unwrap_or(&"").trim();
        if message.is_empty() {
            return Ok(None);
        }
        
        // Read file content for hash
        let full_path = self.config.target_repo.join(&file_path);
        let content = tokio::fs::read_to_string(&full_path).await
            .unwrap_or_default();
        let hash = compute_hash(&content);
        
        let flaw_id = format!("clippy-{}-{}-{}", 
            file_path.display(),
            line_num,
            &hash[..8.min(hash.len())]
        );
        
        Ok(Some(Flaw {
            id: flaw_id,
            file_path,
            line: line_num,
            priority,
            category,
            description: message.to_string(),
            suggestion: None,
            context: String::new(),
            content_hash: hash,
        }))
    }
    
    /// Analyze test coverage
    async fn analyze_test_coverage(&self, entities: &[CodeEntity]) -> Result<Vec<Flaw>, ForgeError> {
        let mut flaws = Vec::new();
        
        // Find public functions without corresponding tests
        for entity in entities.iter().filter(|e| e.kind == EntityKind::Function) {
            let has_test = entities.iter().any(|e| {
                e.kind == EntityKind::Function && 
                e.name.starts_with("test_") &&
                e.content.contains(&entity.name)
            });
            
            if !has_test && entity.name.starts_with("pub ") {
                let flaw = self.create_flaw(
                    entity,
                    FlawCategory::TestGap,
                    "Public function without test coverage".to_string(),
                    format!("Add unit test for '{}' to improve coverage.", entity.name),
                ).await?;
                flaws.push(flaw);
            }
        }
        
        Ok(flaws)
    }
}
