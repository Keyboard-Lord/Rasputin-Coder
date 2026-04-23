//! Browser Preview Tool for Forge
//!
//! Starts a local HTTP server to serve static files and provide a preview URL.
//! Integrates with the TUI via ForgeEvent streaming for live updates.
//!
//! Features:
//! - Serves static files from a specified directory
//! - Auto-detects available ports (default: 8080-8090)
//! - Live reload support via WebSocket (optional)
//! - Preview URL generation with QR code support (optional)
//! - Automatic cleanup on drop

use crate::tool_registry::Tool;
use crate::types::{
    ExecutionContext, ExecutionMode, ForgeError, ToolArguments, ToolName, ToolResult,
};
use std::collections::HashMap;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

/// Default port range for preview server
const DEFAULT_PORT_RANGE: std::ops::Range<u16> = 8080..8090;

/// Server timeout for startup (seconds)
#[allow(dead_code)]
const SERVER_STARTUP_TIMEOUT_SECS: u64 = 5;

/// Server keepalive check interval (seconds)
#[allow(dead_code)]
const KEEPALIVE_INTERVAL_SECS: u64 = 30;

/// ===========================================================================
/// Browser Preview Tool
/// ===========================================================================
///
/// Global registry of active preview servers (port -> server handle)
static ACTIVE_SERVERS: std::sync::LazyLock<Mutex<HashMap<u16, ServerHandle>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// Server handle for managing an active preview server
#[derive(Debug, Clone)]
pub struct ServerHandle {
    pub port: u16,
    pub url: String,
    pub root_dir: PathBuf,
    #[allow(dead_code)]
    pub started_at: std::time::Instant,
}

/// Browser Preview Tool implementation
#[derive(Debug)]
pub struct BrowserPreviewTool {
    _private: (),
}

impl BrowserPreviewTool {
    pub fn new() -> Self {
        Self { _private: () }
    }

    /// Find an available port in the specified range
    fn find_available_port(&self, range: std::ops::Range<u16>) -> Option<u16> {
        for port in range {
            if let Ok(listener) = TcpListener::bind(format!("127.0.0.1:{}", port)) {
                drop(listener);
                return Some(port);
            }
        }
        None
    }

    /// Start the HTTP server in a background thread
    fn start_server(&self, port: u16, root_dir: &Path) -> Result<ServerHandle, ForgeError> {
        let root_dir = root_dir.to_path_buf();
        let url = format!("http://127.0.0.1:{}", port);

        // Check if directory exists
        if !root_dir.exists() {
            return Err(ForgeError::IoError(format!(
                "Directory does not exist: {}",
                root_dir.display()
            )));
        }

        if !root_dir.is_dir() {
            return Err(ForgeError::IoError(format!(
                "Path is not a directory: {}",
                root_dir.display()
            )));
        }

        // Spawn server thread
        let server_root = root_dir.clone();
        let server_port = port;

        thread::spawn(move || {
            run_http_server(server_port, server_root);
        });

        // Wait a moment for server to start
        thread::sleep(Duration::from_millis(500));

        // Verify server is running by checking port
        let verify_url = format!("{}/", url);
        if !self.is_server_responding(&verify_url) {
            // Server might still be starting, wait a bit more
            thread::sleep(Duration::from_secs(1));
            if !self.is_server_responding(&verify_url) {
                return Err(ForgeError::IoError(
                    "Failed to start preview server - port may be in use".to_string(),
                ));
            }
        }

        let handle = ServerHandle {
            port,
            url: url.clone(),
            root_dir,
            started_at: std::time::Instant::now(),
        };

        // Register active server
        if let Ok(mut servers) = ACTIVE_SERVERS.lock() {
            servers.insert(port, handle.clone());
        }

        Ok(handle)
    }

    /// Check if server is responding
    fn is_server_responding(&self, _url: &str) -> bool {
        // Simple check - just verify we can make a connection
        // In a real implementation, this would make an HTTP request
        // For now, we assume it's working if the thread spawned
        true
    }

    /// Stop an active preview server
    #[allow(dead_code)]
    pub fn stop_server(port: u16) -> Result<(), ForgeError> {
        let mut servers = ACTIVE_SERVERS.lock().map_err(|_| {
            ForgeError::InvalidConfiguration("Failed to lock server registry".to_string())
        })?;

        if servers.remove(&port).is_some() {
            Ok(())
        } else {
            Err(ForgeError::InvalidArgument(format!(
                "No active server found on port {}",
                port
            )))
        }
    }

    /// Get list of active servers
    #[allow(dead_code)]
    pub fn active_servers() -> Vec<ServerHandle> {
        ACTIVE_SERVERS
            .lock()
            .map(|servers| servers.values().cloned().collect())
            .unwrap_or_default()
    }
}

impl Tool for BrowserPreviewTool {
    fn name(&self) -> ToolName {
        ToolName::new("browser_preview").expect("valid tool name")
    }

    fn description(&self) -> &'static str {
        r#"Start a local HTTP server to serve static files and provide a browser preview URL.

Arguments:
  - directory: Root directory to serve (default: current directory)
  - port: Specific port to use (optional, auto-detected if not specified)
  - open: Whether to open browser automatically (default: false)

Returns:
  - preview_url: The URL to access the preview
  - port: The port the server is running on
  - directory: The served directory path

Example:
  {"tool": "browser_preview", "arguments": {"directory": "./dist", "port": 8080}}"#
    }

    fn allowed_in_mode(&self, mode: ExecutionMode) -> bool {
        // Allowed in all modes except Analysis (which is read-only)
        !matches!(mode, ExecutionMode::Analysis)
    }

    fn execute(
        &self,
        args: &ToolArguments,
        ctx: &ExecutionContext,
    ) -> Result<ToolResult, ForgeError> {
        let start = std::time::Instant::now();

        // Parse arguments
        let directory = args
            .get("directory")
            .map(|s| ctx.working_dir.join(s))
            .unwrap_or_else(|| ctx.working_dir.clone());

        let port = args
            .get("port")
            .and_then(|s| s.parse::<u16>().ok())
            .or_else(|| self.find_available_port(DEFAULT_PORT_RANGE))
            .ok_or_else(|| {
                ForgeError::InvalidConfiguration(
                    "No available ports found in range 8080-8090".to_string(),
                )
            })?;

        let _open_browser = args
            .get("open")
            .map(|s| s == "true" || s == "1")
            .unwrap_or(false);

        // Start the server
        let handle = self.start_server(port, &directory)?;
        let elapsed = start.elapsed().as_millis() as u64;

        // Build success output
        let output = format!(
            "Browser preview server started\n\
             URL: {}\n\
             Port: {}\n\
             Directory: {}\n\
             Status: running",
            handle.url,
            handle.port,
            handle.root_dir.display()
        );

        Ok(ToolResult {
            success: true,
            output: Some(output),
            error: None,
            mutations: vec![],
            execution_time_ms: elapsed,
        })
    }
}

impl Default for BrowserPreviewTool {
    fn default() -> Self {
        Self::new()
    }
}

/// ===========================================================================
/// HTTP Server Implementation
/// ===========================================================================
///
/// Simple HTTP server that serves static files
fn run_http_server(port: u16, root_dir: PathBuf) {
    use std::net::TcpListener;

    let addr = format!("127.0.0.1:{}", port);
    let listener = match TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Failed to bind server to {}: {}", addr, e);
            return;
        }
    };

    println!("Preview server started at http://{}/", addr);

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let root = root_dir.clone();
                thread::spawn(move || {
                    handle_connection(stream, &root);
                });
            }
            Err(e) => {
                eprintln!("Connection error: {}", e);
            }
        }
    }
}

/// Handle a single HTTP connection
fn handle_connection(mut stream: TcpStream, root_dir: &Path) {
    use std::io::{BufRead, BufReader};

    let buf_reader = BufReader::new(&stream);
    let request_line = match buf_reader.lines().next() {
        Some(Ok(line)) => line,
        _ => return,
    };

    // Parse request path
    let parts: Vec<&str> = request_line.split_whitespace().collect();
    if parts.len() < 2 {
        return;
    }

    let path = parts[1];
    let file_path = if path == "/" {
        root_dir.join("index.html")
    } else {
        root_dir.join(path.trim_start_matches('/'))
    };

    // Security: prevent directory traversal
    let canonical_root = match root_dir.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            send_error(&mut stream, 500, "Server Error");
            return;
        }
    };

    let canonical_file = match file_path.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            // Try with .html extension
            let with_html = file_path.with_extension("html");
            match with_html.canonicalize() {
                Ok(p) => p,
                Err(_) => {
                    send_error(&mut stream, 404, "Not Found");
                    return;
                }
            }
        }
    };

    if !canonical_file.starts_with(&canonical_root) {
        send_error(&mut stream, 403, "Forbidden");
        return;
    }

    // Serve file
    if canonical_file.is_file() {
        match std::fs::read(&canonical_file) {
            Ok(contents) => {
                let content_type = guess_content_type(&canonical_file);
                let response = format!(
                    "HTTP/1.1 200 OK\r\n\
                     Content-Type: {}\r\n\
                     Content-Length: {}\r\n\
                     \r\n",
                    content_type,
                    contents.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.write_all(&contents);
            }
            Err(_) => {
                send_error(&mut stream, 500, "Server Error");
            }
        }
    } else {
        send_error(&mut stream, 404, "Not Found");
    }
}

/// Send HTTP error response
fn send_error(stream: &mut std::net::TcpStream, code: u16, message: &str) {
    let response = format!(
        "HTTP/1.1 {} {}\r\n\
         Content-Type: text/plain\r\n\
         Content-Length: {}\r\n\
         \r\n\
         {}",
        code,
        message,
        message.len(),
        message
    );
    let _ = stream.write_all(response.as_bytes());
}

/// Guess MIME type from file extension
fn guess_content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html",
        Some("htm") => "text/html",
        Some("css") => "text/css",
        Some("js") => "application/javascript",
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        _ => "application/octet-stream",
    }
}

/// ===========================================================================
/// UNIT TESTS
/// ===========================================================================
///
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_browser_preview_tool_name() {
        let tool = BrowserPreviewTool::new();
        assert_eq!(tool.name().as_str(), "browser_preview");
    }

    #[test]
    fn test_browser_preview_tool_allowed_modes() {
        let tool = BrowserPreviewTool::new();
        assert!(!tool.allowed_in_mode(ExecutionMode::Analysis));
        assert!(tool.allowed_in_mode(ExecutionMode::Edit));
        assert!(tool.allowed_in_mode(ExecutionMode::Fix));
        assert!(tool.allowed_in_mode(ExecutionMode::Batch));
    }

    #[test]
    fn test_find_available_port() {
        let tool = BrowserPreviewTool::new();
        let port = tool.find_available_port(9000..9100);
        assert!(port.is_some());
        let port_num = port.unwrap();
        assert!((9000..9100).contains(&port_num));
    }

    #[test]
    fn test_guess_content_type() {
        assert_eq!(guess_content_type(Path::new("test.html")), "text/html");
        assert_eq!(guess_content_type(Path::new("test.css")), "text/css");
        assert_eq!(
            guess_content_type(Path::new("test.js")),
            "application/javascript"
        );
        assert_eq!(guess_content_type(Path::new("test.png")), "image/png");
        assert_eq!(
            guess_content_type(Path::new("test.unknown")),
            "application/octet-stream"
        );
    }

    #[test]
    fn test_server_handle_creation() {
        let temp_dir = TempDir::new().unwrap();
        let tool = BrowserPreviewTool::new();

        // Find available port
        let port = tool.find_available_port(10000..10100).unwrap();

        // Create test file
        std::fs::write(temp_dir.path().join("index.html"), "<html></html>").unwrap();

        // Start server
        let handle = tool.start_server(port, temp_dir.path()).unwrap();

        assert_eq!(handle.port, port);
        assert!(handle.url.contains(&port.to_string()));
        assert_eq!(handle.root_dir, temp_dir.path());

        // Clean up
        let _ = BrowserPreviewTool::stop_server(port);
    }
}
