//! Browser preview server management
//!
//! Handles browser preview servers for web development workflows.

use chrono::Local;

/// A running browser preview server
#[derive(Debug, Clone)]
pub struct PreviewServer {
    pub id: String,
    pub url: String,
    pub process_id: Option<u32>,
    pub workspace_path: String,
    pub started_at: chrono::DateTime<Local>,
}

impl PreviewServer {
    /// Create a new preview server entry
    pub fn new(id: String, url: String, workspace_path: String) -> Self {
        Self {
            id,
            url,
            process_id: None,
            workspace_path: workspace_path.clone(),
            started_at: Local::now(),
        }
    }

    /// Get the port from the URL (if available)
    pub fn port(&self) -> u16 {
        // Parse port from URL like "http://localhost:3000"
        self.url
            .split(':')
            .last()
            .and_then(|p| p.parse().ok())
            .unwrap_or(0)
    }

    /// Get the directory path
    pub fn directory(&self) -> &str {
        &self.workspace_path
    }
}
