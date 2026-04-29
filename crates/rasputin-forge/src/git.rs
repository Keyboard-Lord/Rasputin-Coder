//! Git operations for the Deep Forge
//! Uses git command execution to avoid OpenSSL dependencies

use crate::types::ForgeError;
use std::path::{Path, PathBuf};
use tokio::process::Command;
use tracing::{debug, error, info, warn};

/// Git operations wrapper using command execution
pub struct GitOps {
    base_path: PathBuf,
}

impl GitOps {
    /// Create new GitOps
    pub fn new(base_path: PathBuf) -> Result<Self, ForgeError> {
        info!("[GIT] Initialized at {:?}", base_path);
        Ok(Self { base_path })
    }
    
    /// Stage a file
    pub async fn stage_file(&self, file_path: &Path) -> Result<(), ForgeError> {
        let output = Command::new("git")
            .args(["add", &file_path.to_string_lossy()])
            .current_dir(&self.base_path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .output()
            .await
            .map_err(|e| ForgeError::Io(e))?;
        
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ForgeError::Linter(format!("git add failed: {}", stderr)));
        }
        
        debug!("[GIT] Staged {:?}", file_path);
        Ok(())
    }
    
    /// Stage all changes
    pub async fn stage_all(&self) -> Result<(), ForgeError> {
        let output = Command::new("git")
            .args(["add", "."])
            .current_dir(&self.base_path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .output()
            .await
            .map_err(|e| ForgeError::Io(e))?;
        
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ForgeError::Linter(format!("git add failed: {}", stderr)));
        }
        
        info!("[GIT] Staged all changes");
        Ok(())
    }
    
    /// Commit staged changes
    pub async fn commit(&self, message: &str) -> Result<(), ForgeError> {
        let output = Command::new("git")
            .args(["commit", "-m", message])
            .current_dir(&self.base_path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .output()
            .await
            .map_err(|e| ForgeError::Io(e))?;
        
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // Check if nothing to commit
            if stderr.contains("nothing to commit") {
                info!("[GIT] Nothing to commit");
                return Ok(());
            }
            return Err(ForgeError::Linter(format!("git commit failed: {}", stderr)));
        }
        
        info!("[GIT] Committed: {}", message.lines().next().unwrap_or(""));
        Ok(())
    }
    
    /// Restore file to HEAD state
    pub async fn restore_file(&self, file_path: &Path) -> Result<(), ForgeError> {
        let output = Command::new("git")
            .args(["checkout", "HEAD", "--", &file_path.to_string_lossy()])
            .current_dir(&self.base_path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .output()
            .await
            .map_err(|e| ForgeError::Io(e))?;
        
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("[GIT] Restore failed: {}", stderr);
            return Err(ForgeError::Linter(format!("git checkout failed: {}", stderr)));
        }
        
        info!("[GIT] Restored {:?}", file_path);
        Ok(())
    }
    
    /// Check if working directory is clean
    pub async fn is_clean(&self) -> Result<bool, ForgeError> {
        let output = Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(&self.base_path)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
            .await
            .map_err(|e| ForgeError::Io(e))?;
        
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.trim().is_empty())
    }
    
    /// Get list of modified files
    pub async fn modified_files(&self) -> Result<Vec<PathBuf>, ForgeError> {
        let output = Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(&self.base_path)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .output()
            .await
            .map_err(|e| ForgeError::Io(e))?;
        
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut files = Vec::new();
        
        for line in stdout.lines() {
            if line.len() >= 3 {
                // Parse git status output: XY PATH or XY ORIG_PATH -> PATH
                let path_part = &line[3..];
                if let Some(path) = path_part.split(" -> ").last() {
                    files.push(PathBuf::from(path.trim()));
                }
            }
        }
        
        Ok(files)
    }
    
    /// Create a backup branch before forging
    pub async fn create_backup_branch(&self) -> Result<(), ForgeError> {
        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let branch_name = format!("forge-backup-{}", timestamp);
        
        let output = Command::new("git")
            .args(["branch", &branch_name])
            .current_dir(&self.base_path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .output()
            .await
            .map_err(|e| ForgeError::Io(e))?;
        
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("[GIT] Branch creation warning: {}", stderr);
            // Continue even if branch exists
        }
        
        info!("[GIT] Created backup branch: {}", branch_name);
        Ok(())
    }
}
