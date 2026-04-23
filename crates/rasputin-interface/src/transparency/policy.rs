/// Policy for rendering internal events to user-facing messages
#[derive(Debug, Clone)]
pub struct RenderingPolicy {
    pub show_timings: bool,
    pub show_raw_events: bool,
    pub max_path_length: usize,
    pub max_error_preview: usize,
}

impl Default for RenderingPolicy {
    fn default() -> Self {
        Self {
            show_timings: false,
            show_raw_events: false,
            max_path_length: 60,
            max_error_preview: 120,
        }
    }
}

impl RenderingPolicy {
    pub fn developer() -> Self {
        Self {
            show_timings: true,
            show_raw_events: true,
            ..Default::default()
        }
    }

    pub fn concise() -> Self {
        Self {
            show_timings: false,
            show_raw_events: false,
            max_path_length: 40,
            max_error_preview: 80,
        }
    }
}

/// Built-in message templates
pub struct MessageTemplates;

impl MessageTemplates {
    pub fn reading_file(path: &str) -> String {
        format!("Reading {}...", path)
    }

    pub fn file_read(path: &str, lines: usize) -> String {
        format!("Read {} ({} lines)", path, lines)
    }

    pub fn writing_file(path: &str) -> String {
        format!("Writing {}...", path)
    }

    pub fn file_written(path: &str) -> String {
        format!("Created {}", path)
    }

    pub fn updating_file(path: &str) -> String {
        format!("Updating {}...", path)
    }

    pub fn file_updated(path: &str) -> String {
        format!("Updated {}", path)
    }

    pub fn deleting_file(path: &str) -> String {
        format!("Deleting {}...", path)
    }

    pub fn file_deleted(path: &str) -> String {
        format!("Deleted {}", path)
    }

    pub fn validation_running(stage: Option<&str>) -> String {
        match stage {
            Some(s) => format!("Validating {}...", s),
            None => "Running validation...".to_string(),
        }
    }

    pub fn validation_passed() -> String {
        "Validation passed".to_string()
    }

    pub fn validation_failed(reason: &str) -> String {
        format!("Validation failed: {}", reason)
    }

    pub fn changes_reverted(files: &[std::path::PathBuf]) -> String {
        if files.len() == 1 {
            format!("Changes to {} reverted", files[0].display())
        } else {
            format!("Changes to {} files reverted", files.len())
        }
    }

    pub fn work_completed(files: &[std::path::PathBuf]) -> String {
        if files.is_empty() {
            "Task completed".to_string()
        } else if files.len() == 1 {
            "Task completed (1 file)".to_string()
        } else {
            format!("Task completed ({} files)", files.len())
        }
    }
}
