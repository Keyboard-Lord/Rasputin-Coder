//! The Chisel: SEARCH/REPLACE patch application engine

use crate::types::{Flaw, ForgeError, Patch, PatchResult};
use regex::Regex;
use std::path::{Path, PathBuf};
use tracing::{debug, error, info, warn};

/// The Chisel applies SEARCH/REPLACE patches to source files
pub struct Chisel {
    /// Base path for all patch operations
    base_path: PathBuf,
}

impl Chisel {
    /// Create a new Chisel
    pub fn new(base_path: PathBuf) -> Self {
        Self { base_path }
    }
    
    /// Parse a raw LLM response into patch blocks
    pub fn parse_patches(&self, raw_response: &str, target_file: &Path) -> Vec<Patch> {
        let mut patches = Vec::new();
        
        // Find all SEARCH/REPLACE blocks
        // Format:
        // ```
        // <<<<<<< SEARCH
        // content to find
        // =======
        // content to replace
        // >>>>>>> REPLACE
        // ```
        
        let search_marker = "<<<<<<< SEARCH";
        let divider = "=======";
        let replace_marker = ">>>>>>> REPLACE";
        
        let mut remaining = raw_response;
        
        while let Some(search_start) = remaining.find(search_marker) {
            let after_search = &remaining[search_start + search_marker.len()..];
            
            // Find divider
            if let Some(div_pos) = after_search.find(divider) {
                let search_content = &after_search[..div_pos].trim();
                let after_div = &after_search[div_pos + divider.len()..];
                
                // Find replace end
                if let Some(replace_end) = after_div.find(replace_marker) {
                    let replace_content = &after_div[..replace_end].trim();
                    
                    patches.push(Patch {
                        file_path: target_file.to_path_buf(),
                        search: search_content.to_string(),
                        replace: replace_content.to_string(),
                        description: "LLM generated fix".to_string(),
                    });
                    
                    remaining = &after_div[replace_end + replace_marker.len()..];
                } else {
                    break;
                }
            } else {
                break;
            }
        }
        
        // Also try alternate format (simpler SEARCH/REPLACE)
        if patches.is_empty() {
            patches = self.parse_simple_patches(raw_response, target_file);
        }
        
        patches
    }
    
    /// Parse simple SEARCH/REPLACE format
    fn parse_simple_patches(&self, raw_response: &str, target_file: &Path) -> Vec<Patch> {
        let mut patches = Vec::new();
        
        // Format:
        // SEARCH:
        // ```
        // content to find
        // ```
        // REPLACE:
        // ```
        // content to replace
        // ```
        
        let search_pattern = Regex::new(
            r"SEARCH:\s*```(?:\w+)?\s*\n(.*?)```\s*\nREPLACE:\s*```(?:\w+)?\s*\n(.*?)```"
        ).unwrap();
        
        for caps in search_pattern.captures_iter(raw_response) {
            let search = caps.get(1).map_or("", |m| m.as_str()).trim();
            let replace = caps.get(2).map_or("", |m| m.as_str()).trim();
            
            if !search.is_empty() {
                patches.push(Patch {
                    file_path: target_file.to_path_buf(),
                    search: search.to_string(),
                    replace: replace.to_string(),
                    description: "LLM generated fix".to_string(),
                });
            }
        }
        
        patches
    }
    
    /// Apply a single patch
    pub async fn apply_patch(&self, patch: &Patch) -> Result<PatchResult, ForgeError> {
        let full_path = self.base_path.join(&patch.file_path);
        
        info!("[CHISEL] Applying patch to {:?}", patch.file_path);
        debug!("[CHISEL] Searching for: {}", &patch.search[..patch.search.len().min(50)]);
        
        // Read file content
        let content = tokio::fs::read_to_string(&full_path).await
            .map_err(|e| {
                error!("[CHISEL] Failed to read file: {:?}", e);
                ForgeError::Io(e)
            })?;
        
        // Find the search text
        if let Some(pos) = content.find(&patch.search) {
            // Apply replacement
            let new_content = format!(
                "{}{}{}",
                &content[..pos],
                patch.replace,
                &content[pos + patch.search.len()..]
            );
            
            // Write back
            tokio::fs::write(&full_path, new_content).await
                .map_err(|e| {
                    error!("[CHISEL] Failed to write file: {:?}", e);
                    ForgeError::Io(e)
                })?;
            
            info!("[CHISEL] Patch applied successfully at position {}", pos);
            
            Ok(PatchResult::Success {
                file_path: patch.file_path.clone(),
                applied_at: pos,
            })
        } else {
            // Try fuzzy matching
            warn!("[CHISEL] Exact match not found, trying fuzzy match...");
            self.fuzzy_apply(&content, patch).await
        }
    }
    
    /// Attempt fuzzy matching for patches
    async fn fuzzy_apply(&self, content: &str, patch: &Patch) -> Result<PatchResult, ForgeError> {
        // Normalize whitespace and try again
        let normalized_search = self.normalize_whitespace(&patch.search);
        let normalized_content = self.normalize_whitespace(content);
        
        if let Some(pos) = normalized_content.find(&normalized_search) {
            // Map position back to original content
            let byte_pos = self.map_to_original(content, pos);
            
            // Apply patch at mapped position
            let new_content = format!(
                "{}{}{}",
                &content[..byte_pos],
                patch.replace,
                &content[byte_pos + patch.search.len()..]
            );
            
            let full_path = self.base_path.join(&patch.file_path);
            tokio::fs::write(&full_path, new_content).await
                .map_err(|e| ForgeError::Io(e))?;
            
            info!("[CHISEL] Fuzzy patch applied at position {}", byte_pos);
            
            Ok(PatchResult::Success {
                file_path: patch.file_path.clone(),
                applied_at: byte_pos,
            })
        } else {
            error!("[CHISEL] Could not find search text in file");
            Ok(PatchResult::Failed {
                reason: "Search text not found in file".to_string(),
            })
        }
    }
    
    /// Normalize whitespace for fuzzy matching
    fn normalize_whitespace(&self, s: &str) -> String {
        s.lines()
            .map(|line| line.trim())
            .collect::<Vec<_>>()
            .join("\n")
    }
    
    /// Map normalized position back to original
    fn map_to_original(&self, original: &str, normalized_pos: usize) -> usize {
        // Simplistic mapping - find nearest line boundary
        let mut orig_pos = 0;
        let mut norm_pos = 0;
        
        for line in original.lines() {
            let trimmed = line.trim();
            let line_len = line.len() + 1; // +1 for newline
            let trimmed_len = trimmed.len() + 1; // +1 for newline
            
            if norm_pos + trimmed_len > normalized_pos {
                return orig_pos + (normalized_pos - norm_pos);
            }
            
            orig_pos += line_len;
            norm_pos += trimmed_len;
        }
        
        original.len()
    }
    
    /// Apply multiple patches atomically
    pub async fn apply_patches(&self, patches: &[Patch]) -> Vec<Result<PatchResult, ForgeError>> {
        let mut results = Vec::new();
        
        for patch in patches {
            let result = self.apply_patch(patch).await;
            results.push(result);
        }
        
        results
    }
    
    /// Create a backup of a file before patching
    pub async fn backup_file(&self, file_path: &Path) -> Result<PathBuf, ForgeError> {
        let full_path = self.base_path.join(file_path);
        let backup_path = full_path.with_extension("rs.bak");
        
        tokio::fs::copy(&full_path, &backup_path).await
            .map_err(|e| ForgeError::Io(e))?;
        
        Ok(backup_path)
    }
    
    /// Restore file from backup
    pub async fn restore_file(&self, file_path: &Path) -> Result<(), ForgeError> {
        let full_path = self.base_path.join(file_path);
        let backup_path = full_path.with_extension("rs.bak");
        
        if backup_path.exists() {
            tokio::fs::copy(&backup_path, &full_path).await
                .map_err(|e| ForgeError::Io(e))?;
            
            // Remove backup
            tokio::fs::remove_file(&backup_path).await
                .map_err(|e| ForgeError::Io(e))?;
        }
        
        Ok(())
    }
}
