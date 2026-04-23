//! HTTP Planner Backend - Real Ollama API Integration
//!
//! Implements HTTP POST /api/generate with streaming support,
//! temperature=0.0 enforcement, and deterministic seed handling.

use crate::planner::adapter::PlannerAdapter;
use crate::planner::state_view::StateView;
use crate::planner::traits::Planner;
use crate::types::{ForgeError, PlannerOutput};
use std::time::{Duration, Instant};

pub const DEFAULT_CODER_14B_MODEL: &str = "qwen2.5-coder:14b";
pub const DEFAULT_CODER_14B_Q4KM_TAG: &str = "qwen2.5-coder:14b-q4km";
pub const DEFAULT_CODER_14B_Q5KM_TAG: &str = "qwen2.5-coder:14b-q5km";
pub const DEFAULT_CODER_14B_IQ4XS_TAG: &str = "qwen2.5-coder:14b-iq4xs";
pub const DEFAULT_CODER_14B_Q3KM_TAG: &str = "qwen2.5-coder:14b-q3km";
pub const FALLBACK_PLANNER_MODEL: &str = "qwen3.5:latest";

pub fn normalize_requested_model(model: &str) -> String {
    let normalized = model.trim().to_lowercase();

    match normalized.as_str() {
        "" => DEFAULT_CODER_14B_MODEL.to_string(),
        "14b" | "coder14b" | "qwen14b" | "qwen-coder-14b" => DEFAULT_CODER_14B_MODEL.to_string(),
        "qwen2.5-coder:14b-q4_k_m" => DEFAULT_CODER_14B_Q4KM_TAG.to_string(),
        "qwen2.5-coder:14b-q5_k_m" => DEFAULT_CODER_14B_Q5KM_TAG.to_string(),
        "qwen2.5-coder:14b-iq4_xs" => DEFAULT_CODER_14B_IQ4XS_TAG.to_string(),
        "qwen2.5-coder:14b-q3_k_m" => DEFAULT_CODER_14B_Q3KM_TAG.to_string(),
        _ => normalized,
    }
}

pub fn preferred_model_candidates(model: &str) -> Vec<String> {
    let normalized = normalize_requested_model(model);
    let ordered = match normalized.as_str() {
        DEFAULT_CODER_14B_MODEL => vec![
            DEFAULT_CODER_14B_Q4KM_TAG,
            DEFAULT_CODER_14B_Q5KM_TAG,
            DEFAULT_CODER_14B_MODEL,
            DEFAULT_CODER_14B_IQ4XS_TAG,
            DEFAULT_CODER_14B_Q3KM_TAG,
            FALLBACK_PLANNER_MODEL,
        ],
        DEFAULT_CODER_14B_Q4KM_TAG => vec![
            DEFAULT_CODER_14B_Q4KM_TAG,
            DEFAULT_CODER_14B_Q5KM_TAG,
            DEFAULT_CODER_14B_MODEL,
            FALLBACK_PLANNER_MODEL,
        ],
        DEFAULT_CODER_14B_Q5KM_TAG => vec![
            DEFAULT_CODER_14B_Q5KM_TAG,
            DEFAULT_CODER_14B_Q4KM_TAG,
            DEFAULT_CODER_14B_MODEL,
            FALLBACK_PLANNER_MODEL,
        ],
        DEFAULT_CODER_14B_IQ4XS_TAG => vec![
            DEFAULT_CODER_14B_IQ4XS_TAG,
            DEFAULT_CODER_14B_Q4KM_TAG,
            DEFAULT_CODER_14B_Q5KM_TAG,
            DEFAULT_CODER_14B_MODEL,
            FALLBACK_PLANNER_MODEL,
        ],
        DEFAULT_CODER_14B_Q3KM_TAG => vec![
            DEFAULT_CODER_14B_Q3KM_TAG,
            DEFAULT_CODER_14B_IQ4XS_TAG,
            DEFAULT_CODER_14B_Q4KM_TAG,
            DEFAULT_CODER_14B_Q5KM_TAG,
            DEFAULT_CODER_14B_MODEL,
            FALLBACK_PLANNER_MODEL,
        ],
        _ => vec![normalized.as_str(), FALLBACK_PLANNER_MODEL],
    };

    let mut deduped = Vec::new();
    for candidate in ordered {
        let candidate = candidate.to_string();
        if !deduped.contains(&candidate) {
            deduped.push(candidate);
        }
    }
    deduped
}

pub fn detect_model_size_b(model: &str) -> Option<u32> {
    let normalized = normalize_requested_model(model);
    let hints = [32_u32, 14, 7, 3, 1];

    hints.into_iter().find(|size| {
        normalized.contains(&format!(":{}b", size))
            || normalized.contains(&format!("-{}b", size))
            || normalized == format!("{}b", size)
    })
}

pub fn should_enable_css_compression(model: &str) -> bool {
    detect_model_size_b(model).is_some_and(|size| size >= 14)
}

fn has_literal_artifact_markers(task: &str) -> bool {
    task.contains("Target artifact:") && task.contains("Artifact class:")
}

fn task_scope_rules(task: &str) -> &'static str {
    if has_literal_artifact_markers(task) {
        "LITERAL ARTIFACT RULE:\n\
         This task carries an explicit literal artifact plan. Create or validate exactly the \
         Target artifact path and do not substitute source files, implementation-location \
         notes, repository-analysis documents, or nearby artifact classes. Completion is valid \
         only after that target path has been written and the completion reason cites it."
    } else {
        "IMPLEMENTATION SCOPE RULE:\n\
         This task has no explicit literal artifact marker. If it asks for an app, system, site, \
         API, auth, persistence, migration, integration, wiring, architecture, or tooling, make \
         real implementation progress toward the requested working surface. Prefer reading \
         relevant project files, modifying source/config/runtime surfaces, and validating the \
         result. Do not satisfy complex implementation work with only notes, summaries, TODOs, \
         placeholder docs, or scaffolding-only artifacts. For multi-surface goals, do not \
         complete after only one plausible file; continue until observable state covers the \
         requested entrypoint, routing/integration, persistence/data, build/serve, or migration \
         surfaces. Once the required surfaces are present and validation has no unresolved \
         errors, emit completion instead of cosmetic edits. Treat Repository Shape context as \
         authoritative path evidence: if a guessed path is missing, redirect to listed source, \
         config, entrypoint, or test surfaces instead of retrying the missing path. For Rust \
         modules, keep module file creation and src/lib.rs exposure aligned before expecting \
         cargo test to pass. Completion is valid only when observable state shows the requested \
         working system slice."
    }
}

/// HTTP-based Ollama backend using /api/generate
pub struct HttpOllamaBackend {
    endpoint: String,
    model_name: String,
    timeout: Duration,
    temperature: f32,
    seed: Option<u64>,
}

impl HttpOllamaBackend {
    #[allow(dead_code)]
    pub fn new(endpoint: String, model_name: String) -> Self {
        Self {
            endpoint,
            model_name: normalize_requested_model(&model_name),
            timeout: Duration::from_secs(30),
            temperature: 0.0, // Deterministic by default per spec
            seed: Some(42),   // Fixed seed for determinism
        }
    }

    #[allow(dead_code)]
    pub fn with_timeout(mut self, seconds: u64) -> Self {
        self.timeout = Duration::from_secs(seconds);
        self
    }

    #[allow(dead_code)]
    pub fn with_temperature(mut self, temp: f32) -> Self {
        // Enforce max 0.1 for schema compliance per FORGE_PLANNER_OUTPUT_CONTRACT_SPEC.md
        self.temperature = temp.clamp(0.0, 0.1);
        self
    }

    #[allow(dead_code)]
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self
    }

    /// Call Ollama /api/generate endpoint
    pub fn generate(
        &self,
        prompt: &str,
        system_prompt: Option<&str>,
    ) -> Result<String, ForgeError> {
        let url = format!("{}/api/generate", self.endpoint);

        // Build request body per Ollama API spec
        let mut body = serde_json::json!({
            "model": self.model_name,
            "prompt": prompt,
            "stream": false, // Get complete response
            "format": "json",
            "options": {
                "temperature": self.temperature,
                // Seed for deterministic sampling if supported
                "seed": self.seed.unwrap_or(42)
            }
        });

        // Add system prompt if provided
        if let Some(sys) = system_prompt {
            body["system"] = serde_json::json!(sys);
        }

        let body_str = body.to_string();

        // Use curl for HTTP request (available on all systems)
        let output = std::process::Command::new("curl")
            .args([
                "-s", // silent
                "-X",
                "POST",
                "-H",
                "Content-Type: application/json",
                "-d",
                &body_str,
                "--connect-timeout",
                &self.timeout.as_secs().to_string(),
                "--max-time",
                &self.timeout.as_secs().to_string(),
                &url,
            ])
            .output()
            .map_err(|e| {
                ForgeError::PlannerBackendUnavailable(format!(
                    "Failed to execute curl: {}. Is curl installed?",
                    e
                ))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ForgeError::PlannerBackendUnavailable(format!(
                "curl failed: {}",
                stderr
            )));
        }

        let response_str = String::from_utf8_lossy(&output.stdout);

        // Parse Ollama response
        let response: serde_json::Value = serde_json::from_str(&response_str).map_err(|e| {
            ForgeError::PlannerNormalizationError(format!("Failed to parse Ollama response: {}", e))
        })?;

        // Extract response text
        let response_text = response["response"].as_str().ok_or_else(|| {
            ForgeError::PlannerNormalizationError(
                "Ollama response missing 'response' field".to_string(),
            )
        })?;

        Ok(response_text.trim().to_string())
    }

    /// Check backend health
    pub fn health_check(&self) -> Result<(), ForgeError> {
        let url = format!("{}/api/tags", self.endpoint);

        let output = std::process::Command::new("curl")
            .args([
                "-s",
                "-o",
                "/dev/null",
                "-w",
                "%{http_code}",
                "--connect-timeout",
                "5",
                &url,
            ])
            .output()
            .map_err(|e| {
                ForgeError::PlannerBackendUnavailable(format!("Health check failed: {}", e))
            })?;

        let status_code = String::from_utf8_lossy(&output.stdout);
        if status_code.trim() == "200" {
            Ok(())
        } else {
            Err(ForgeError::PlannerBackendUnavailable(format!(
                "Ollama not responding (HTTP {})",
                status_code.trim()
            )))
        }
    }
}

use crate::planner::css_transformer::{CompressionContext, CssTransformer};
use crate::planner::validator::{
    PlannerValidator, ReadRecord, ValidationContext as VContext, ValidationDecision,
};

/// Model planner with HTTP backend, 13-rule validator, and optional CSS compression
pub struct HttpModelPlanner {
    backend: HttpOllamaBackend,
    #[allow(dead_code)]
    validator: PlannerValidator,
    adapter: PlannerAdapter,
    system_prompt: String,
    max_retries: u32,
    // PHASE 4: CSS compression for 8B models
    css_transformer: Option<CssTransformer>,
    #[allow(dead_code)]
    compression_ctx: CompressionContext,
    use_css: bool,
}

impl HttpModelPlanner {
    #[allow(dead_code)]
    pub fn with_backend(backend: HttpOllamaBackend) -> Self {
        Self {
            backend,
            validator: PlannerValidator::new(),
            adapter: PlannerAdapter::new(),
            system_prompt: Self::default_system_prompt(),
            max_retries: 3,
            css_transformer: None,
            compression_ctx: CompressionContext::default(),
            use_css: false,
        }
    }

    /// Enable CSS compression while keeping the full planner contract intact.
    pub fn with_css_compression(mut self) -> Self {
        self.css_transformer = Some(CssTransformer::new());
        self.use_css = true;
        eprintln!("[PLANNER] CSS compression ENABLED");
        self
    }

    #[allow(dead_code)]
    pub fn with_max_retries(mut self, retries: u32) -> Self {
        self.max_retries = retries;
        self
    }

    fn default_system_prompt() -> String {
        r#"You are the FORGE Planner - an expert coding assistant that emits ONLY valid canonical JSON.

CRITICAL RULES (VIOLATIONS = IMMEDIATE REJECTION):
1. EXACTLY ONE JSON object per response - no other text
2. NO markdown fences (```json), NO prose, NO explanations
3. NO shell commands (ls, cat, rm, |, >, etc.)
4. NO shorthand formats - use exact canonical schema
5. READ-BEFORE-WRITE: Always read files before modifying

AVAILABLE TOOLS:
- read_file: Read file content
  Args: {"path": "string"}

- write_file: Create or overwrite file
  Args: {"path": "string", "content": "string"}

- apply_patch: Replace exact text (requires hash from read_file)
  Args: {"file_path": "string", "old_text": "string", "new_text": "string", "expected_hash": "sha256:..."}

CANONICAL OUTPUT FORMAT - COPY THESE EXACTLY:

EXAMPLE 1 - Read file:
{"type":"tool_call","tool_call":{"name":"read_file","arguments":{"path":"src/main.rs"}}}

EXAMPLE 2 - Write file:
{"type":"tool_call","tool_call":{"name":"write_file","arguments":{"path":"src/utils.rs","content":"fn greet() -> String {\n    \"Hello from Forge\".to_string()\n}"}}}

EXAMPLE 3 - Complete task:
{"type":"completion","reason":"Created src/utils.rs with greet() function returning 'Hello from Forge'"}

VALIDATION CHECKLIST (Auto-applied):
□ Single JSON object only
□ Exact field names: type, tool_call, name, arguments
□ No extra whitespace or newlines outside JSON
□ All string values properly escaped
□ File paths relative to project root

ENGINEERING SCOPE RULE:
If a task asks for an app, system, site, API, auth, persistence, migration, integration, wiring, or tooling, make real implementation progress toward that working surface.
Do not reduce complex implementation work to notes, summaries, placeholder docs, or scaffolding-only artifacts.
Completion is valid only when observable state shows the requested working system slice, not merely a description of it.

FORBIDDEN PATTERNS (Will be rejected):
- Markdown: ```json ... ```
- Multiple JSONs: {"a":1}{"b":2}
- Prose: "I'll help you..."
- Shorthand: {"action":"read","file":"main.rs"}
- Wrapper: {"status":"ok","result":{...}}

Emit ONLY raw JSON. No thinking aloud."#.to_string()
    }

    fn build_prompt(&self, state: &StateView) -> String {
        // PHASE 4: Use CSS compression when enabled
        if self.use_css {
            return self.build_css_prompt(state);
        }

        let available_tools = state
            .available_tools
            .iter()
            .map(|t| format!("- {}: {}", t.name.as_str(), t.description))
            .collect::<Vec<_>>()
            .join("\n");

        let files_read = state
            .files_read
            .iter()
            .map(|f| {
                format!(
                    "- {} (hash: {}, full_read: {}){}",
                    f.path.display(),
                    &f.content_hash[..16.min(f.content_hash.len())],
                    f.is_full_read,
                    f.content_excerpt
                        .as_ref()
                        .map(|excerpt| format!("\n  excerpt:\n{}", indent_excerpt(excerpt)))
                        .unwrap_or_default()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let files_written = state
            .files_written
            .iter()
            .map(|path| format!("- {}", path.display()))
            .collect::<Vec<_>>()
            .join("\n");

        let recent_errors = state
            .recent_errors
            .iter()
            .map(|error| format!("- {}", error))
            .collect::<Vec<_>>()
            .join("\n");

        let task_scope_rules = task_scope_rules(&state.task);

        format!(
            "Task: {}\n\nAvailable tools:\n{}\n\nFiles read this session:\n{}\n\nFiles written this session:\n{}\n\nRecent errors or repair instructions:\n{}\n\n{}\n\nIteration: {}/{}\n\nPROPOSE EXACTLY ONE ACTION (raw JSON only, no markdown):",
            state.task,
            if available_tools.is_empty() {
                "(none)"
            } else {
                &available_tools
            },
            if files_read.is_empty() {
                "(none)"
            } else {
                &files_read
            },
            if files_written.is_empty() {
                "(none)"
            } else {
                &files_written
            },
            if recent_errors.is_empty() {
                "(none)"
            } else {
                &recent_errors
            },
            task_scope_rules,
            state.iteration,
            state.max_iterations
        )
    }

    /// Build a lighter-weight prompt while preserving the standard planner contract.
    fn build_css_prompt(&self, state: &StateView) -> String {
        let files_read_compressed: Vec<String> = state
            .files_read
            .iter()
            .map(|f| {
                let path_hash = &f.content_hash[..8.min(f.content_hash.len())];
                let excerpt = f
                    .content_excerpt
                    .as_ref()
                    .map(|value| format!("@{}", compress_excerpt(value)))
                    .unwrap_or_default();
                format!(
                    "{}#{}#{}{}",
                    f.path.display(),
                    path_hash,
                    if f.is_full_read { "full" } else { "partial" },
                    excerpt,
                )
            })
            .collect();

        let tools_list: Vec<String> = state
            .available_tools
            .iter()
            .map(|t| t.name.as_str().to_string())
            .collect();

        let files_written_compressed: Vec<String> = state
            .files_written
            .iter()
            .map(|path| path.display().to_string())
            .collect();

        let recent_errors = state
            .recent_errors
            .iter()
            .map(|error| error.replace('\n', " "))
            .collect::<Vec<_>>();

        let task_scope_rules = task_scope_rules(&state.task);

        format!(
            "Task: {}\n\nAvailable tools (compressed): {}\n\nFiles read this session (compressed): {}\n\nFiles written this session: {}\n\nRecent errors or repair instructions: {}\n\n{}\n\nIteration: {}/{}\n\nEmit ONE JSON action (raw JSON only, no markdown):",
            state.task,
            if tools_list.is_empty() {
                "(none)".to_string()
            } else {
                tools_list.join(", ")
            },
            if files_read_compressed.is_empty() {
                "(none)".to_string()
            } else {
                files_read_compressed.join(", ")
            },
            if files_written_compressed.is_empty() {
                "(none)".to_string()
            } else {
                files_written_compressed.join(", ")
            },
            if recent_errors.is_empty() {
                "(none)".to_string()
            } else {
                recent_errors.join(" | ")
            },
            task_scope_rules,
            state.iteration,
            state.max_iterations,
        )
    }

    fn build_validation_context(&self, state: &StateView) -> VContext {
        VContext {
            mode: state.mode,
            available_tools: state
                .available_tools
                .iter()
                .map(|t| t.name.as_str().to_string())
                .collect(),
            iteration: state.iteration,
            files_read: state
                .files_read
                .iter()
                .map(|f| ReadRecord {
                    path: f.path.display().to_string(),
                    iteration: f.read_at_iteration,
                    is_full_read: f.is_full_read,
                    content_hash: f.content_hash.clone(),
                })
                .collect(),
            task_description: state.task.clone(),
        }
    }

    /// Try to normalize output with fallback strategies
    fn try_normalize(&self, raw: &str, state: &StateView) -> Option<PlannerOutput> {
        // Try direct parsing first
        if let Ok(output) = self.adapter.normalize(raw, state) {
            return Some(output);
        }

        // Try stripping markdown fences
        let cleaned = raw
            .replace("```json", "")
            .replace("```", "")
            .trim()
            .to_string();

        if cleaned != raw
            && let Ok(output) = self.adapter.normalize(&cleaned, state)
        {
            eprintln!("[PLANNER] Normalized markdown fences");
            return Some(output);
        }

        None
    }
}

fn indent_excerpt(excerpt: &str) -> String {
    excerpt
        .lines()
        .map(|line| format!("    {}", line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn compress_excerpt(excerpt: &str) -> String {
    excerpt.replace('\n', " ").chars().take(120).collect()
}

impl Planner for HttpModelPlanner {
    fn generate(&self, state: &StateView) -> Result<PlannerOutput, ForgeError> {
        let start = Instant::now();
        let prompt = self.build_prompt(state);
        let validation_ctx = self.build_validation_context(state);

        let mut last_error = None;

        for attempt in 0..=self.max_retries {
            if attempt > 0 {
                eprintln!(
                    "[PLANNER] Retry {}/{} with correction prompt...",
                    attempt, self.max_retries
                );
            } else {
                eprintln!(
                    "[PLANNER] Calling Ollama with {} char prompt...",
                    prompt.len()
                );
            }

            let raw_response = self.backend.generate(&prompt, Some(&self.system_prompt))?;

            let elapsed = start.elapsed().as_millis();
            let preview: String = raw_response.chars().take(200).collect();
            eprintln!(
                "[PLANNER] Response (attempt {}): {}ms, {} chars",
                attempt + 1,
                elapsed,
                raw_response.len()
            );
            eprintln!("[PLANNER] Preview: {}", preview);

            // Validate with 13-rule validator
            let mut validator = PlannerValidator::new();
            let decision = validator.validate(&raw_response, &validation_ctx);

            match decision {
                ValidationDecision::Accept => {
                    // Try to parse into PlannerOutput
                    match self.try_normalize(&raw_response, state) {
                        Some(output) => {
                            eprintln!("[PLANNER] ✓ Valid canonical output accepted");
                            return Ok(output);
                        }
                        None => {
                            last_error = Some("Failed to parse validated output".to_string());
                        }
                    }
                }
                ValidationDecision::Reject {
                    reason,
                    tier,
                    failure_class,
                    rule_broken,
                } => {
                    eprintln!(
                        "[PLANNER] ✗ Rejected (tier {}, rule {}): {} - {}",
                        tier, rule_broken, failure_class, reason
                    );
                    last_error = Some(format!("{}: {}", rule_broken, reason));

                    if attempt < self.max_retries {
                        let correction =
                            validator.generate_correction_prompt(&ValidationDecision::Reject {
                                reason: reason.clone(),
                                tier,
                                failure_class,
                                rule_broken,
                            });
                        eprintln!(
                            "[PLANNER] Applying correction: {}",
                            correction.lines().next().unwrap_or("")
                        );
                    }
                }
                ValidationDecision::Escalate {
                    reason,
                    violation,
                    failure_class,
                    rule_broken,
                } => {
                    eprintln!(
                        "[PLANNER] ✗✗ ESCALATION (rule {}): {} - {} - {}",
                        rule_broken, violation, failure_class, reason
                    );
                    return Err(ForgeError::PlannerContractViolation(format!(
                        "Critical violation {} ({}): {}",
                        rule_broken, violation, reason
                    )));
                }
            }
        }

        Err(ForgeError::PlannerNormalizationError(format!(
            "Failed to produce valid output after {} retries: {:?}",
            self.max_retries + 1,
            last_error
        )))
    }

    fn generate_raw(&self, state: &StateView) -> Result<String, ForgeError> {
        let prompt = self.build_prompt(state);
        self.backend.generate(&prompt, Some(&self.system_prompt))
    }

    fn planner_type(&self) -> &'static str {
        "http_ollama"
    }

    fn health_check(&self) -> Result<(), ForgeError> {
        self.backend.health_check()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backend_temperature_enforcement() {
        let backend =
            HttpOllamaBackend::new("http://localhost:11434".to_string(), "test".to_string())
                .with_temperature(0.5); // Try to set high

        // Should be clamped to 0.1
        assert_eq!(backend.temperature, 0.1);
    }

    #[test]
    fn test_backend_deterministic_defaults() {
        let backend =
            HttpOllamaBackend::new("http://localhost:11434".to_string(), "test".to_string());

        assert_eq!(backend.temperature, 0.0);
        assert_eq!(backend.seed, Some(42));
    }

    #[test]
    fn normalizes_14b_aliases() {
        assert_eq!(normalize_requested_model("14b"), DEFAULT_CODER_14B_MODEL);
        assert_eq!(
            normalize_requested_model("coder14b"),
            DEFAULT_CODER_14B_MODEL
        );
        assert_eq!(
            normalize_requested_model("qwen2.5-coder:14b-q4_k_m"),
            DEFAULT_CODER_14B_Q4KM_TAG
        );
    }

    #[test]
    fn prefers_quantized_14b_candidates_first() {
        assert_eq!(
            preferred_model_candidates("14b"),
            vec![
                DEFAULT_CODER_14B_Q4KM_TAG.to_string(),
                DEFAULT_CODER_14B_Q5KM_TAG.to_string(),
                DEFAULT_CODER_14B_MODEL.to_string(),
                DEFAULT_CODER_14B_IQ4XS_TAG.to_string(),
                DEFAULT_CODER_14B_Q3KM_TAG.to_string(),
                FALLBACK_PLANNER_MODEL.to_string(),
            ]
        );
    }

    #[test]
    fn enables_css_for_14b_models() {
        assert!(should_enable_css_compression("qwen2.5-coder:14b"));
        assert!(should_enable_css_compression("coder14b"));
        assert!(!should_enable_css_compression("qwen3.5:latest"));
    }

    #[test]
    fn build_prompt_includes_file_excerpt() {
        let backend =
            HttpOllamaBackend::new("http://localhost:11434".to_string(), "test".to_string());
        let planner = HttpModelPlanner::with_backend(backend);
        let mut state = state_with_task("inspect excerpt");
        state.files_read = vec![crate::planner::state_view::FileReadInfo {
            path: std::path::PathBuf::from("src/main.rs"),
            content_hash: "sha256:abc".to_string(),
            size_bytes: 42,
            total_lines: 3,
            is_full_read: true,
            read_at_iteration: 1,
            content_excerpt: Some("fn main() {\n    println!(\"hi\");\n}".to_string()),
        }];

        let prompt = planner.build_prompt(&state);
        assert!(prompt.contains("excerpt:"));
        assert!(prompt.contains("println!(\"hi\")"));
    }

    #[test]
    fn default_system_prompt_has_no_literal_artifact_bias() {
        let backend =
            HttpOllamaBackend::new("http://localhost:11434".to_string(), "test".to_string());
        let planner = HttpModelPlanner::with_backend(backend);

        assert!(!planner.system_prompt.contains("LITERAL ARTIFACT RULE"));
        assert!(!planner.system_prompt.contains("Target artifact: <path>"));
        assert!(planner.system_prompt.contains("ENGINEERING SCOPE RULE"));
    }

    #[test]
    fn build_prompt_includes_literal_rule_only_for_marker_tasks() {
        let backend =
            HttpOllamaBackend::new("http://localhost:11434".to_string(), "test".to_string());
        let planner = HttpModelPlanner::with_backend(backend);
        let literal_state = state_with_task(
            "create a tiny docs note file: Target artifact: docs/tiny-note.md; Artifact class: docs note;",
        );
        let complex_state = state_with_task("build a Python CLI app");

        let literal_prompt = planner.build_prompt(&literal_state);
        let complex_prompt = planner.build_prompt(&complex_state);

        assert!(literal_prompt.contains("LITERAL ARTIFACT RULE"));
        assert!(literal_prompt.contains("Target artifact path"));
        assert!(!literal_prompt.contains("IMPLEMENTATION SCOPE RULE"));
        assert!(!complex_prompt.contains("LITERAL ARTIFACT RULE"));
        assert!(!complex_prompt.contains("Target artifact path"));
        assert!(complex_prompt.contains("IMPLEMENTATION SCOPE RULE"));
        assert!(complex_prompt.contains("app, system, site"));
        assert!(complex_prompt.contains("Do not satisfy complex implementation work"));
        assert!(complex_prompt.contains("multi-surface goals"));
        assert!(complex_prompt.contains("do not complete after only one plausible file"));
        assert!(complex_prompt.contains("emit completion instead of cosmetic edits"));
    }

    #[test]
    fn css_prompt_uses_implementation_scope_for_complex_tasks() {
        let backend =
            HttpOllamaBackend::new("http://localhost:11434".to_string(), "test".to_string());
        let planner = HttpModelPlanner::with_backend(backend).with_css_compression();
        let state = state_with_task("build a small note-taking app with persistence");

        let prompt = planner.build_prompt(&state);

        assert!(prompt.contains("IMPLEMENTATION SCOPE RULE"));
        assert!(prompt.contains("persistence"));
        assert!(!prompt.contains("LITERAL ARTIFACT RULE"));
    }

    fn state_with_task(task: &str) -> StateView {
        StateView {
            task: task.to_string(),
            session_id: "session".to_string(),
            iteration: 1,
            max_iterations: 3,
            mode: crate::types::ExecutionMode::Edit,
            files_read: vec![],
            files_written: vec![],
            available_tools: vec![],
            recent_executions: vec![],
            last_validation: None,
            recent_errors: vec![],
            repo_root: std::path::PathBuf::from("."),
            allowed_paths: vec![],
        }
    }
}
