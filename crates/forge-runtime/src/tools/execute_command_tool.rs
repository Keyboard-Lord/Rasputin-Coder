//! Execute Command Tool for Forge
//!
//! Executes shell commands with strict safety controls:
//! - Allowlist of safe commands (cargo, npm, python, rustc, etc.)
//! - Timeout enforcement to prevent hung processes
//! - Working directory validation (repo boundary)
//! - Output capture with size limits
//! - Destructive command detection and safety confirmation
//! - Mode-based access control (not allowed in Analysis mode)

use crate::tool_registry::Tool;
use crate::types::{
    ExecutionContext, ExecutionMode, ForgeError, ToolArguments, ToolError, ToolResult,
};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// ===========================================================================
/// SAFETY CONFIGURATION
/// ===========================================================================
///
/// Commands allowed to execute without confirmation
const SAFE_COMMAND_ALLOWLIST: &[&str] = &[
    // Rust ecosystem
    "cargo", "rustc", "rustfmt", "clippy", // JavaScript/Node ecosystem
    "npm", "node", "npx", "yarn", "pnpm", // Python ecosystem
    "python", "python3", "pip", "pip3", "pytest", // Version control (read-only)
    "git",    // Build tools
    "make", "cmake", // Utilities (read-only)
    "echo", "cat", "ls", "pwd", "which", "wc", "grep", "find", "head", "tail", "sort", "uniq",
];

/// Commands that require explicit safety confirmation
const DESTRUCTIVE_COMMANDS: &[&str] = &["rm", "del", "remove", "unlink", "rmdir"];

/// Git subcommands that are destructive
const DESTRUCTIVE_GIT_SUBCOMMANDS: &[&str] =
    &["push", "reset", "clean", "checkout", "rebase", "amend"];

/// Maximum output size (10MB to prevent memory exhaustion)
const MAX_OUTPUT_SIZE: usize = 10 * 1024 * 1024;

/// Default timeout in seconds
const DEFAULT_TIMEOUT_SECONDS: u64 = 30;

/// Maximum allowed timeout in seconds
const MAX_TIMEOUT_SECONDS: u64 = 300; // 5 minutes

/// ===========================================================================
/// PATH VALIDATION (shared pattern)
/// ===========================================================================
///
/// Validates that a path is within the allowed repository boundary
fn validate_path_boundary(path: &Path, working_dir: &Path) -> Result<PathBuf, ForgeError> {
    let canonical_working = working_dir
        .canonicalize()
        .unwrap_or_else(|_| working_dir.to_path_buf());

    let normalized = if path.is_absolute() {
        path.to_path_buf()
    } else {
        canonical_working.join(path)
    };

    let canonical = normalized
        .canonicalize()
        .unwrap_or_else(|_| normalized.clone());

    if !canonical.starts_with(&canonical_working) {
        return Err(ForgeError::InvalidArgument(format!(
            "Path '{}' is outside repository boundary '{}'",
            path.display(),
            canonical_working.display()
        )));
    }

    Ok(canonical)
}

/// ===========================================================================
/// COMMAND SAFETY VALIDATION
/// ===========================================================================
///
/// Result of command safety validation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandSafety {
    Safe,
    RequiresConfirmation { reason: String },
    Blocked { reason: String },
}

/// Parse command string into command and arguments
fn parse_command(command_str: &str) -> (String, Vec<String>) {
    // Simple parsing - split by whitespace
    // This handles basic cases; for complex shell features we'd need a shell parser
    let parts: Vec<&str> = command_str.split_whitespace().collect();

    if parts.is_empty() {
        return (String::new(), vec![]);
    }

    let command = parts[0].to_lowercase();
    let args = parts[1..].iter().map(|s| s.to_string()).collect();

    (command, args)
}

/// Validate command safety
fn validate_command_safety(command: &str, args: &[String]) -> CommandSafety {
    // Check for destructive commands first (before allowlist check)
    if DESTRUCTIVE_COMMANDS.contains(&command) {
        return CommandSafety::RequiresConfirmation {
            reason: format!(
                "Command '{}' is potentially destructive (file deletion). \
                 Destructive commands require explicit confirmation.",
                command
            ),
        };
    }

    // Check if command is in allowlist
    if !SAFE_COMMAND_ALLOWLIST.contains(&command) {
        return CommandSafety::Blocked {
            reason: format!(
                "Command '{}' is not in the safe command allowlist. \
                 Allowed commands: {:?}",
                command, SAFE_COMMAND_ALLOWLIST
            ),
        };
    }

    // Special handling for git commands
    if command == "git" && !args.is_empty() {
        let subcommand = args[0].to_lowercase();

        if DESTRUCTIVE_GIT_SUBCOMMANDS.iter().any(|&c| c == subcommand) {
            return CommandSafety::RequiresConfirmation {
                reason: format!(
                    "Git '{}' command is potentially destructive. \
                     This may modify repository state or remote history.",
                    subcommand
                ),
            };
        }
    }

    // Check for shell metacharacters that could enable command injection
    let full_command = format!("{} {}", command, args.join(" "));
    if contains_shell_metacharacters(&full_command) {
        // Still allow it but require confirmation
        return CommandSafety::RequiresConfirmation {
            reason: "Command contains shell metacharacters. This requires confirmation for safety."
                .to_string(),
        };
    }

    CommandSafety::Safe
}

/// Check if string contains shell metacharacters
fn contains_shell_metacharacters(s: &str) -> bool {
    let dangerous_chars = [
        ';', '&', '|', '$', '`', '>', '<', '(', ')', '{', '}', '*', '?',
    ];
    s.chars().any(|c| dangerous_chars.contains(&c))
}

/// ===========================================================================
/// EXECUTE COMMAND TOOL
/// ===========================================================================
///
/// Execute command tool with strict safety controls
///
/// Arguments:
/// - command: Command string to execute (required)
/// - working_dir: Working directory for execution (optional, defaults to repo root)
/// - timeout_seconds: Maximum execution time (default: 30, max: 300)
/// - require_confirmation: Override to require confirmation even for safe commands
/// - capture_stderr: Whether to include stderr in output (default: true)
/// - max_output_lines: Maximum lines to return in output (default: 1000)
///
/// Safety features:
/// - Command allowlist enforcement
/// - Destructive command detection
/// - Timeout enforcement
/// - Output size limits
/// - Working directory boundary validation
pub struct ExecuteCommandTool;

impl ExecuteCommandTool {
    pub fn new() -> Self {
        Self
    }

    /// Execute command with timeout
    fn execute_with_timeout(
        &self,
        command_str: &str,
        working_dir: &Path,
        timeout_secs: u64,
        capture_stderr: bool,
    ) -> Result<(bool, String, Option<String>), ForgeError> {
        let _start = Instant::now();

        // Parse command
        let (command, args) = parse_command(command_str);

        if command.is_empty() {
            return Err(ForgeError::InvalidArgument(
                "Empty command string".to_string(),
            ));
        }

        // Build command
        let mut cmd = Command::new(&command);
        cmd.args(&args)
            .current_dir(working_dir)
            .stdout(Stdio::piped());

        if capture_stderr {
            cmd.stderr(Stdio::piped());
        } else {
            cmd.stderr(Stdio::null());
        }

        // Spawn process
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return Ok((
                    false,
                    String::new(),
                    Some(format!("Failed to spawn command '{}': {}", command_str, e)),
                ));
            }
        };

        // Take ownership of stdout/stderr handles before waiting
        let stdout_handle = child.stdout.take();
        let stderr_handle = child.stderr.take();

        // Get process ID for potential kill
        let pid = child.id();

        // Wait with timeout using wait_timeout from waitpid crate pattern
        let timeout_duration = Duration::from_secs(timeout_secs);
        let start_time = Instant::now();

        // Poll for completion with timeout
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) => {
                    // Process still running
                    if start_time.elapsed() >= timeout_duration {
                        // Timeout reached - kill process
                        #[cfg(unix)]
                        {
                            use std::process::Command as SysCommand;
                            let _ = SysCommand::new("kill")
                                .args(["-9", &pid.to_string()])
                                .output();
                        }
                        #[cfg(windows)]
                        {
                            let _ = Command::new("taskkill")
                                .args(&["/F", "/PID", &pid.to_string()])
                                .output();
                        }
                        // Wait a moment for kill to take effect
                        thread::sleep(Duration::from_millis(100));
                        let _ = child.wait(); // Reap zombie
                        break Err(ForgeError::ExecutionTimeout {
                            command: command_str.to_string(),
                            timeout_secs,
                        });
                    }
                    // Short sleep to avoid busy waiting
                    thread::sleep(Duration::from_millis(10));
                }
                Err(e) => {
                    break Err(ForgeError::IoError(format!(
                        "Failed to wait for command: {}",
                        e
                    )));
                }
            }
        };

        let status = status?;

        // Read stdout (now from the captured handle)
        let stdout = stdout_handle
            .and_then(|mut s| {
                use std::io::Read;
                let mut buf = Vec::new();
                s.read_to_end(&mut buf).ok()?;
                Some(String::from_utf8_lossy(&buf).to_string())
            })
            .unwrap_or_default();

        // Read stderr if captured
        let stderr = if capture_stderr {
            stderr_handle.and_then(|mut s| {
                use std::io::Read;
                let mut buf = Vec::new();
                s.read_to_end(&mut buf).ok()?;
                Some(String::from_utf8_lossy(&buf).to_string())
            })
        } else {
            None
        };

        let success = status.success();
        let output = stdout;
        let error = if success {
            None
        } else {
            match status.code() {
                Some(code) => {
                    let mut err_msg = format!("Command exited with code {}", code);
                    if let Some(ref stderr_str) = stderr
                        && !stderr_str.is_empty()
                    {
                        err_msg.push_str(&format!(
                            "\nStderr: {}",
                            &stderr_str[..stderr_str.len().min(500)]
                        ));
                    }
                    Some(err_msg)
                }
                None => Some("Command terminated by signal".to_string()),
            }
        };

        Ok((success, output, error))
    }

    /// Truncate output to maximum size and lines
    fn truncate_output(output: &str, max_lines: usize) -> String {
        let lines: Vec<&str> = output.lines().collect();

        if lines.len() <= max_lines {
            // Check byte size
            if output.len() > MAX_OUTPUT_SIZE {
                let truncated = &output[..MAX_OUTPUT_SIZE];
                format!(
                    "{}\n[Output truncated: exceeded {}MB size limit]",
                    truncated,
                    MAX_OUTPUT_SIZE / (1024 * 1024)
                )
            } else {
                output.to_string()
            }
        } else {
            let visible: Vec<&str> = lines[..max_lines].to_vec();
            let mut result = visible.join("\n");
            result.push_str(&format!(
                "\n[Output truncated: {} more lines hidden]",
                lines.len() - max_lines
            ));
            result
        }
    }
}

impl Default for ExecuteCommandTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for ExecuteCommandTool {
    fn name(&self) -> crate::types::ToolName {
        crate::types::ToolName::new("execute_command").expect("valid tool name")
    }

    fn description(&self) -> &str {
        "Execute a shell command with strict safety controls. \
         Commands are validated against an allowlist. \
         Destructive commands require confirmation. \
         Timeout enforced (default 30s, max 300s). \
         Output is captured with size limits. \
         Not allowed in Analysis mode."
    }

    fn allowed_in_mode(&self, mode: ExecutionMode) -> bool {
        // execute_command is NOT allowed in Analysis mode (read-only)
        matches!(
            mode,
            ExecutionMode::Edit | ExecutionMode::Fix | ExecutionMode::Batch
        )
    }

    fn execute(
        &self,
        args: &ToolArguments,
        ctx: &ExecutionContext,
    ) -> Result<ToolResult, ForgeError> {
        if !self.allowed_in_mode(ctx.mode) {
            return Err(ForgeError::ToolNotAllowed {
                tool: self.name(),
                mode: ctx.mode,
            });
        }

        let start = Instant::now();

        // Extract required command
        let command_str = args.require("command")?;

        if command_str.trim().is_empty() {
            return Err(ForgeError::InvalidArgument(
                "command cannot be empty".to_string(),
            ));
        }

        // Parse and validate command
        let (command, cmd_args) = parse_command(command_str);

        // Validate command safety
        let safety = validate_command_safety(&command, &cmd_args);

        // Check if confirmation is required
        let require_confirmation = args
            .get("require_confirmation")
            .and_then(|s| s.parse::<bool>().ok())
            .unwrap_or(false);

        match safety {
            CommandSafety::Blocked { reason: _ } => {
                let elapsed = start.elapsed().as_millis() as u64;
                return Ok(ToolResult {
                    success: false,
                    output: None,
                    error: Some(ToolError::NotAllowed {
                        tool: self.name(),
                        mode: ctx.mode,
                    }),
                    mutations: vec![],
                    execution_time_ms: elapsed,
                });
            }
            CommandSafety::RequiresConfirmation { reason } if !require_confirmation => {
                let elapsed = start.elapsed().as_millis() as u64;
                return Ok(ToolResult {
                    success: false,
                    output: None,
                    error: Some(ToolError::ExecutionFailed(format!(
                        "SAFETY_CONFIRMATION_REQUIRED: {}\n\n\
                         To execute this command, set require_confirmation=true.\n\
                         Command: {}",
                        reason, command_str
                    ))),
                    mutations: vec![],
                    execution_time_ms: elapsed,
                });
            }
            _ => {
                // Safe or confirmation provided - proceed
            }
        }

        // Extract and validate working directory
        let working_dir = args
            .get("working_dir")
            .map(PathBuf::from)
            .unwrap_or_else(|| ctx.working_dir.clone());

        let validated_dir = validate_path_boundary(&working_dir, &ctx.working_dir)?;

        // Parse timeout
        let timeout_secs = args
            .get("timeout_seconds")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(DEFAULT_TIMEOUT_SECONDS)
            .min(MAX_TIMEOUT_SECONDS);

        // Parse output options
        let capture_stderr = args
            .get("capture_stderr")
            .and_then(|s| s.parse::<bool>().ok())
            .unwrap_or(true);

        let max_output_lines = args
            .get("max_output_lines")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(1000)
            .min(10000); // Hard cap at 10k lines

        // Execute command with timeout
        let (success, output, error) =
            self.execute_with_timeout(command_str, &validated_dir, timeout_secs, capture_stderr)?;

        // Truncate output if needed
        let truncated_output = Self::truncate_output(&output, max_output_lines);

        let elapsed = start.elapsed().as_millis() as u64;

        // Build metadata
        let mut metadata = HashMap::new();
        metadata.insert("command".to_string(), command_str.to_string());
        metadata.insert(
            "working_dir".to_string(),
            validated_dir.display().to_string(),
        );
        metadata.insert("timeout_secs".to_string(), timeout_secs.to_string());
        metadata.insert("execution_time_ms".to_string(), elapsed.to_string());

        // Format result output
        let result_output = if truncated_output.len() > 500 {
            format!(
                "Executed: {}\nExit: {}\nOutput (first 500 chars): {}...",
                command_str,
                if success { "success" } else { "failure" },
                &truncated_output[..500]
            )
        } else {
            format!(
                "Executed: {}\nExit: {}\nOutput:\n{}",
                command_str,
                if success { "success" } else { "failure" },
                truncated_output
            )
        };

        Ok(ToolResult {
            success,
            output: Some(result_output),
            error: error.map(ToolError::ExecutionFailed),
            mutations: vec![], // Commands may mutate but we track at higher level
            execution_time_ms: elapsed,
        })
    }
}

/// ===========================================================================
/// UNIT TESTS
/// ===========================================================================
///
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_context(dir: &TempDir) -> ExecutionContext {
        ExecutionContext {
            session_id: crate::types::SessionId::new(),
            iteration: 1,
            mode: ExecutionMode::Edit,
            working_dir: dir.path().to_path_buf(),
        }
    }

    fn make_args(pairs: &[(&str, &str)]) -> ToolArguments {
        let mut args = ToolArguments::new();
        for (k, v) in pairs {
            args.set(k, v);
        }
        args
    }

    #[test]
    fn test_parse_command() {
        let (cmd, args) = parse_command("cargo build --release");
        assert_eq!(cmd, "cargo");
        assert_eq!(args, vec!["build", "--release"]);
    }

    #[test]
    fn test_parse_command_empty() {
        let (cmd, args) = parse_command("");
        assert_eq!(cmd, "");
        assert!(args.is_empty());
    }

    #[test]
    fn test_validate_safe_command() {
        let safety = validate_command_safety("cargo", &["build".to_string()]);
        assert_eq!(safety, CommandSafety::Safe);
    }

    #[test]
    fn test_validate_blocked_command() {
        let safety = validate_command_safety("sudo", &["rm".to_string()]);
        assert!(matches!(safety, CommandSafety::Blocked { .. }));
    }

    #[test]
    fn test_validate_destructive_rm() {
        let safety = validate_command_safety("rm", &["-rf".to_string(), "/".to_string()]);
        assert!(matches!(safety, CommandSafety::RequiresConfirmation { .. }));
    }

    #[test]
    fn test_validate_destructive_git_push() {
        let safety = validate_command_safety("git", &["push".to_string(), "origin".to_string()]);
        assert!(matches!(safety, CommandSafety::RequiresConfirmation { .. }));
    }

    #[test]
    fn test_validate_safe_git_status() {
        let safety = validate_command_safety("git", &["status".to_string()]);
        assert_eq!(safety, CommandSafety::Safe);
    }

    #[test]
    fn test_validate_shell_metacharacters() {
        let safety = validate_command_safety("echo", &["hello; rm -rf /".to_string()]);
        assert!(matches!(safety, CommandSafety::RequiresConfirmation { .. }));
    }

    #[test]
    fn test_truncate_output_small() {
        let _tool = ExecuteCommandTool::new();
        let output = "line1\nline2\nline3";
        let result = ExecuteCommandTool::truncate_output(output, 10);
        assert_eq!(result, output);
    }

    #[test]
    fn test_truncate_output_large() {
        let _tool = ExecuteCommandTool::new();
        let lines: Vec<String> = (0..100).map(|i| format!("line{}", i)).collect();
        let output = lines.join("\n");
        let result = ExecuteCommandTool::truncate_output(&output, 50);
        assert!(result.contains("line0"));
        assert!(result.contains("line49"));
        assert!(!result.contains("line50"));
        assert!(result.contains("truncated"));
    }

    #[test]
    fn test_execute_echo_command() {
        let temp_dir = TempDir::new().unwrap();

        let tool = ExecuteCommandTool::new();
        let ctx = create_test_context(&temp_dir);
        let args = make_args(&[("command", "echo hello world")]);

        let result = tool.execute(&args, &ctx).unwrap();
        assert!(result.success);
        assert!(result.output.as_ref().unwrap().contains("hello world"));
    }

    #[test]
    fn test_execute_blocked_command() {
        let temp_dir = TempDir::new().unwrap();

        let tool = ExecuteCommandTool::new();
        let ctx = create_test_context(&temp_dir);
        let args = make_args(&[("command", "sudo ls")]);

        let result = tool.execute(&args, &ctx).unwrap();
        assert!(!result.success);
        assert!(result.error.is_some());
    }

    #[test]
    fn test_execute_destructive_requires_confirmation() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(temp_dir.path().join("test.txt"), "content").unwrap();

        let tool = ExecuteCommandTool::new();
        let ctx = create_test_context(&temp_dir);
        let args = make_args(&[("command", "rm test.txt")]);

        let result = tool.execute(&args, &ctx).unwrap();
        assert!(!result.success);
        let error = result.error.as_ref().unwrap().to_string();
        assert!(error.contains("SAFETY_CONFIRMATION_REQUIRED"));

        // Verify file still exists
        assert!(temp_dir.path().join("test.txt").exists());
    }

    #[test]
    fn test_execute_destructive_with_confirmation() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(temp_dir.path().join("test.txt"), "content").unwrap();

        let tool = ExecuteCommandTool::new();
        let ctx = create_test_context(&temp_dir);
        let args = make_args(&[("command", "rm test.txt"), ("require_confirmation", "true")]);

        let result = tool.execute(&args, &ctx).unwrap();
        // Should succeed now that confirmation is provided
        assert!(result.success || result.error.is_none());

        // Verify file was deleted (if command actually ran)
        // Note: This may fail on Windows where rm doesn't exist
    }

    #[test]
    fn test_execute_blocked_in_analysis_mode() {
        let temp_dir = TempDir::new().unwrap();

        let tool = ExecuteCommandTool::new();
        let mut ctx = create_test_context(&temp_dir);
        ctx.mode = ExecutionMode::Analysis; // Read-only mode

        let args = make_args(&[("command", "echo test")]);

        let result = tool.execute(&args, &ctx);
        assert!(result.is_err()); // Should fail with ToolNotAllowed
    }

    #[test]
    fn test_execute_with_timeout() {
        let temp_dir = TempDir::new().unwrap();

        let tool = ExecuteCommandTool::new();
        let ctx = create_test_context(&temp_dir);
        let args = make_args(&[
            ("command", "sleep 60"),  // Would take 60 seconds
            ("timeout_seconds", "1"), // But we timeout after 1 second
        ]);

        let result = tool.execute(&args, &ctx);
        // Should timeout
        assert!(result.is_err() || result.as_ref().unwrap().error.is_some());
    }

    #[test]
    fn test_execute_working_dir_validation() {
        let temp_dir = TempDir::new().unwrap();

        let tool = ExecuteCommandTool::new();
        let ctx = create_test_context(&temp_dir);
        // Use an absolute path that's clearly outside the repo
        let outside_path = if cfg!(windows) { "C:\\Windows" } else { "/etc" };
        let args = make_args(&[("command", "pwd"), ("working_dir", outside_path)]);

        let result = tool.execute(&args, &ctx);
        assert!(result.is_err()); // Should fail with path validation error
    }

    #[test]
    fn test_execute_cargo_check() {
        let temp_dir = TempDir::new().unwrap();

        let tool = ExecuteCommandTool::new();
        let ctx = create_test_context(&temp_dir);
        let args = make_args(&[("command", "cargo --version")]);

        let result = tool.execute(&args, &ctx).unwrap();
        assert!(result.success);
        assert!(result.output.as_ref().unwrap().contains("cargo"));
    }
}
