//! Local Ollama Qwen-Coder integration for air-gapped operation

use crate::types::{Flaw, ForgeConfig, ForgeError, FlawCategory};
use reqwest::{Client, Response};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::time::timeout;
use tracing::{debug, error, info, warn};
use futures::StreamExt;

/// Ollama API client for local LLM communication
pub struct OllamaClient {
    client: Client,
    config: ForgeConfig,
}

/// Request to Ollama generate endpoint
#[derive(Serialize)]
struct GenerateRequest {
    model: String,
    prompt: String,
    stream: bool,
    options: Option<Value>,
}

/// Response from Ollama generate endpoint
#[derive(Deserialize, Debug)]
struct GenerateResponse {
    response: String,
    done: bool,
    #[serde(default)]
    context: Option<Vec<u64>>,
}

impl OllamaClient {
    /// Create new Ollama client
    pub fn new(config: ForgeConfig) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(config.ollama_timeout))
            .build()
            .expect("Failed to build HTTP client");
        
        Self { client, config }
    }
    
    /// Generate a fix for a specific flaw
    pub async fn generate_fix(&self, flaw: &Flaw, file_content: &str) -> Result<String, ForgeError> {
        let prompt = self.build_fix_prompt(flaw, file_content);
        
        info!("[OLLAMA] Generating fix for flaw {} in {:?}", flaw.id, flaw.file_path);
        
        let response = self.generate(&prompt).await?;
        
        // Extract SEARCH/REPLACE blocks from response
        let patches = self.extract_patches(&response);
        
        if patches.is_empty() {
            warn!("[OLLAMA] No SEARCH/REPLACE blocks found in response");
            // Return raw response for manual inspection
            Ok(response)
        } else {
            Ok(patches)
        }
    }
    
    /// Build the prompt for fix generation
    fn build_fix_prompt(&self, flaw: &Flaw, file_content: &str) -> String {
        format!(r#"You are an expert Rust code reviewer and refactoring assistant.

TASK: Fix the following code issue.

FILE: {:?}
LINE: {}
SEVERITY: {:?}
PRIORITY: {}/100

ISSUE DESCRIPTION:
{}

SUGGESTED FIX:
{}

CURRENT CODE CONTEXT:
```rust
{}
```

INSTRUCTIONS:
1. Provide EXACTLY ONE fix using standard SEARCH/REPLACE format
2. The SEARCH block must match the code exactly (including whitespace)
3. The REPLACE block should contain the corrected code
4. Only modify what's necessary to fix the issue
5. Ensure the code compiles and follows Rust best practices

OUTPUT FORMAT (use exactly):
```
<<<<<<< SEARCH
[exact code to find]
=======
[corrected code]
>>>>>>> REPLACE
```

SEARCH/REPLACE BLOCK:"#, 
            flaw.file_path,
            flaw.line,
            flaw.category,
            flaw.priority,
            flaw.description,
            flaw.suggestion.as_ref().unwrap_or(&"None provided".to_string()),
            self.extract_context(file_content, flaw.line, 10)
        )
    }
    
    /// Extract context around a specific line
    fn extract_context(&self, content: &str, target_line: usize, context_lines: usize) -> String {
        let lines: Vec<&str> = content.lines().collect();
        let start = target_line.saturating_sub(context_lines + 1);
        let end = (target_line + context_lines).min(lines.len());
        
        lines[start..end].join("\n")
    }
    
    /// Send generation request to Ollama
    async fn generate(&self, prompt: &str) -> Result<String, ForgeError> {
        let request = GenerateRequest {
            model: self.config.model.clone(),
            prompt: prompt.to_string(),
            stream: true,
            options: Some(json!({
                "temperature": 0.2,
                "top_p": 0.9,
                "top_k": 40,
                "repeat_penalty": 1.1,
            })),
        };
        
        let url = &self.config.ollama_endpoint;
        
        debug!("[OLLAMA] Sending request to {}", url);
        
        // Stream response for real-time feedback
        let response = timeout(
            Duration::from_secs(self.config.ollama_timeout),
            self.client.post(url)
                .json(&request)
                .send()
        ).await
            .map_err(|_| ForgeError::Ollama("Request timeout".to_string()))?
            .map_err(|e| ForgeError::Ollama(format!("HTTP error: {}", e)))?;
        
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ForgeError::Ollama(
                format!("Ollama error ({}): {}", status, body)
            ));
        }
        
        // Stream and collect response
        self.stream_response(response).await
    }
    
    /// Stream response from Ollama and collect
    async fn stream_response(&self, response: Response) -> Result<String, ForgeError> {
        let mut stream = response.bytes_stream();
        let mut full_response = String::new();
        
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| ForgeError::Ollama(format!("Stream error: {}", e)))?;
            
            // Parse NDJSON (newline-delimited JSON)
            let text = String::from_utf8_lossy(&chunk);
            
            for line in text.lines() {
                if line.trim().is_empty() {
                    continue;
                }
                
                match serde_json::from_str::<GenerateResponse>(line) {
                    Ok(parsed) => {
                        // Stream token to stdout for visual feedback
                        print!("{}", parsed.response);
                        std::io::Write::flush(&mut std::io::stdout()).ok();
                        
                        full_response.push_str(&parsed.response);
                        
                        if parsed.done {
                            println!(); // Newline after streaming
                        }
                    }
                    Err(e) => {
                        debug!("[OLLAMA] Parse error for line '{}': {}", line, e);
                    }
                }
            }
        }
        
        Ok(full_response)
    }
    
    /// Extract SEARCH/REPLACE blocks from LLM response
    fn extract_patches(&self, response: &str) -> String {
        let mut result = String::new();
        
        // Find all SEARCH/REPLACE blocks
        let search_marker = "<<<<<<< SEARCH";
        let replace_marker = ">>>>>>> REPLACE";
        
        let mut start = 0;
        while let Some(pos) = response[start..].find(search_marker) {
            let absolute_pos = start + pos;
            
            // Find end of this block
            if let Some(end_pos) = response[absolute_pos..].find(replace_marker) {
                let block_end = absolute_pos + end_pos + replace_marker.len();
                let block = &response[absolute_pos..block_end];
                result.push_str(block);
                result.push('\n');
                result.push('\n');
                start = block_end;
            } else {
                break;
            }
        }
        
        if result.is_empty() {
            // Return original if no blocks found
            response.to_string()
        } else {
            result
        }
    }
    
    /// Generate comprehensive flaw analysis
    pub async fn analyze_repository(&self, repo_summary: &str) -> Result<String, ForgeError> {
        let prompt = format!(r#"Analyze the following Rust repository for code quality issues:

REPOSITORY SUMMARY:
{}

Identify the top 10 most critical issues that should be fixed, prioritized by:
1. Compiler errors
2. Security vulnerabilities
3. Performance bottlenecks
4. Code complexity
5. Missing tests

For each issue, provide:
- File location
- Line number
- Issue description
- Suggested fix approach

ANALYSIS:"#, repo_summary);
        
        self.generate(&prompt).await
    }
    
    /// Check if Ollama is available
    pub async fn health_check(&self) -> Result<bool, ForgeError> {
        let health_url = self.config.ollama_endpoint.replace("/api/generate", "/api/tags");
        
        match self.client.get(&health_url).send().await {
            Ok(response) => Ok(response.status().is_success()),
            Err(_) => Ok(false),
        }
    }
}
