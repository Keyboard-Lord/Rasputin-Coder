//! Ollama integration module

use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, warn};

pub const DEFAULT_CODER_14B_MODEL: &str = "qwen2.5-coder:14b";
pub const DEFAULT_CODER_14B_Q4KM_TAG: &str = "qwen2.5-coder:14b-q4km";
pub const DEFAULT_CODER_14B_Q5KM_TAG: &str = "qwen2.5-coder:14b-q5km";
pub const DEFAULT_CODER_14B_IQ4XS_TAG: &str = "qwen2.5-coder:14b-iq4xs";
pub const DEFAULT_CODER_14B_Q3KM_TAG: &str = "qwen2.5-coder:14b-q3km";
pub const FALLBACK_PLANNER_MODEL: &str = "qwen3.5:latest";

pub struct OllamaClient {
    endpoint: String,
    client: Client,
}

impl OllamaClient {
    /// Get the configured endpoint URL
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

#[derive(Debug, Clone, Deserialize)]
struct TagsResponse {
    models: Vec<ModelInfo>,
}

#[derive(Debug, Clone, Deserialize)]
struct ModelInfo {
    name: String,
    details: Option<ModelDetails>,
}

#[derive(Debug, Clone, Deserialize)]
struct ModelDetails {
    parameter_size: Option<String>,
    quantization_level: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatApiMessage>,
    stream: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatApiMessage {
    role: String,
    content: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ChatResponse {
    message: Option<ChatApiMessage>,
}

impl OllamaClient {
    pub fn local_default() -> Self {
        Self::new("http://127.0.0.1:11434".to_string())
    }

    pub fn new(endpoint: String) -> Self {
        assert!(
            endpoint.starts_with("http://127.0.0.1:")
                || endpoint.starts_with("http://[::1]:")
                || endpoint.starts_with("http://localhost:"),
            "Ollama endpoint must be loopback-only"
        );
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_default();

        Self { endpoint, client }
    }

    pub fn security_posture(&self) -> &'static str {
        "Loopback-only HTTP client; remote Ollama endpoints are rejected"
    }

    /// Check if Ollama is reachable and list installed models
    pub async fn health_check(&self) -> Result<HealthStatus> {
        let url = format!("{}/api/tags", self.endpoint);
        debug!("Health check: GET {}", url);

        match self.client.get(&url).send().await {
            Ok(response) => {
                if !response.status().is_success() {
                    let error = format!("HTTP {}", response.status());
                    warn!("Ollama health check failed: {}", error);
                    return Ok(HealthStatus {
                        connected: false,
                        models: vec![],
                        model_cards: vec![],
                        error: Some(error),
                    });
                }

                match response.json::<TagsResponse>().await {
                    Ok(data) => {
                        let model_cards: Vec<InstalledModelCard> = data
                            .models
                            .into_iter()
                            .map(|model| InstalledModelCard {
                                name: model.name,
                                parameter_size: model
                                    .details
                                    .as_ref()
                                    .and_then(|details| details.parameter_size.clone()),
                                quantization_level: model
                                    .details
                                    .as_ref()
                                    .and_then(|details| details.quantization_level.clone()),
                            })
                            .collect();
                        let models: Vec<String> =
                            model_cards.iter().map(|model| model.name.clone()).collect();

                        info!("Ollama connected, {} models installed", models.len());
                        debug!("Installed models: {:?}", models);

                        Ok(HealthStatus {
                            connected: true,
                            models,
                            model_cards,
                            error: None,
                        })
                    }
                    Err(e) => {
                        let error = format!("Failed to parse response: {}", e);
                        error!("{}", error);
                        Ok(HealthStatus {
                            connected: false,
                            models: vec![],
                            model_cards: vec![],
                            error: Some(error),
                        })
                    }
                }
            }
            Err(e) => {
                let error = format!("Connection failed: {}", e);
                warn!("Ollama health check failed: {}", error);
                Ok(HealthStatus {
                    connected: false,
                    models: vec![],
                    model_cards: vec![],
                    error: Some(error),
                })
            }
        }
    }

    pub async fn list_model_cards(&self) -> Result<Vec<InstalledModelCard>> {
        let health = self.health_check().await?;
        if health.connected {
            Ok(health.model_cards)
        } else {
            Err(anyhow::anyhow!("Ollama not connected: {:?}", health.error))
        }
    }

    /// Verify if a specific model is installed
    pub async fn verify_model(&self, model: &str) -> Result<ModelVerification> {
        let health = self.health_check().await?;

        if !health.connected {
            return Ok(ModelVerification {
                resolved_model: None,
                exact_match: false,
                installed_models: vec![],
                error: health.error,
            });
        }

        let resolved_model = resolve_model_name(model, &health.model_cards);
        let exact_match = resolved_model.as_deref() == Some(model);
        let is_available = resolved_model.is_some();

        match resolved_model.as_deref() {
            Some(active) if exact_match => info!("Model '{}' verified locally", active),
            Some(active) => info!("Model '{}' resolved to installed '{}'", model, active),
            None => warn!("Model '{}' not found in local Ollama", model),
        }

        Ok(ModelVerification {
            resolved_model: resolved_model.clone(),
            exact_match,
            installed_models: health.models.clone(),
            error: if is_available {
                None
            } else {
                Some(build_model_error(model, &health.model_cards))
            },
        })
    }

    /// Send chat request and get response
    pub async fn chat(&self, model: &str, messages: &[ChatMessage]) -> Result<String> {
        let url = format!("{}/api/chat", self.endpoint);

        let api_messages: Vec<ChatApiMessage> = messages
            .iter()
            .map(|m| ChatApiMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect();

        let request = ChatRequest {
            model: model.to_string(),
            messages: api_messages,
            stream: false,
        };

        info!(
            "Sending chat request to {} with model {}",
            self.endpoint, model
        );

        let response = self.client.post(&url).json(&request).send().await?;

        if !response.status().is_success() {
            let text = response.text().await?;
            return Err(anyhow::anyhow!("Ollama error: {}", text));
        }

        let data: ChatResponse = response.json().await?;

        if let Some(message) = data.message {
            Ok(message.content)
        } else {
            Err(anyhow::anyhow!("No message in response"))
        }
    }
}

#[derive(Debug, Clone)]
pub struct HealthStatus {
    pub connected: bool,
    pub models: Vec<String>,
    pub model_cards: Vec<InstalledModelCard>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct InstalledModelCard {
    pub name: String,
    pub parameter_size: Option<String>,
    pub quantization_level: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ModelVerification {
    pub resolved_model: Option<String>,
    pub exact_match: bool,
    pub installed_models: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

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

pub fn should_enable_css_compression(model: &str) -> bool {
    let normalized = normalize_requested_model(model);
    normalized.contains(":14b")
        || normalized.contains("-14b")
        || normalized.contains(":32b")
        || normalized.contains("-32b")
}

pub fn model_preference_rank(model: &str) -> usize {
    preferred_model_candidates(DEFAULT_CODER_14B_MODEL)
        .iter()
        .position(|candidate| candidate.eq_ignore_ascii_case(model))
        .unwrap_or(usize::MAX)
}

fn resolve_model_name(requested: &str, installed_models: &[InstalledModelCard]) -> Option<String> {
    let normalized = normalize_requested_model(requested);

    if let Some(exact) = installed_models
        .iter()
        .find(|model| model.name.eq_ignore_ascii_case(&normalized))
    {
        return Some(exact.name.clone());
    }

    for candidate in preferred_model_candidates(&normalized) {
        if let Some(installed) = installed_models.iter().find(|model| {
            let lower = model.name.to_lowercase();
            lower == candidate
                || (!candidate.contains(':') && lower.starts_with(&format!("{}:", candidate)))
        }) {
            return Some(installed.name.clone());
        }
    }

    if let Some(installed) = resolve_quantized_base_fallback(&normalized, installed_models) {
        return Some(installed);
    }

    if normalized.starts_with("qwen") {
        let mut qwen_models = installed_models
            .iter()
            .filter(|model| model.name.to_lowercase().starts_with("qwen"))
            .collect::<Vec<_>>();
        qwen_models.sort_by_key(|model| (model_preference_rank(&model.name), model.name.clone()));
        if let Some(best) = qwen_models.into_iter().next() {
            return Some(best.name.clone());
        }
    }

    None
}

fn resolve_quantized_base_fallback(
    requested: &str,
    installed_models: &[InstalledModelCard],
) -> Option<String> {
    let (base_model, quantization_level) = match requested {
        DEFAULT_CODER_14B_Q4KM_TAG => (DEFAULT_CODER_14B_MODEL, Some("Q4_K_M")),
        DEFAULT_CODER_14B_Q5KM_TAG => (DEFAULT_CODER_14B_MODEL, Some("Q5_K_M")),
        DEFAULT_CODER_14B_IQ4XS_TAG => (DEFAULT_CODER_14B_MODEL, Some("IQ4_XS")),
        DEFAULT_CODER_14B_Q3KM_TAG => (DEFAULT_CODER_14B_MODEL, Some("Q3_K_M")),
        _ => (requested, None),
    };

    installed_models
        .iter()
        .find(|model| {
            model.name.eq_ignore_ascii_case(base_model)
                && quantization_level
                    .is_none_or(|expected| model.quantization_level.as_deref() == Some(expected))
        })
        .map(|model| model.name.clone())
}

fn build_available_models(installed_models: &[InstalledModelCard]) -> Vec<String> {
    installed_models
        .iter()
        .map(|model| model.name.clone())
        .collect()
}

fn build_model_error(requested: &str, installed_models: &[InstalledModelCard]) -> String {
    let available_models = build_available_models(installed_models);
    if available_models.is_empty() {
        return format!(
            "Model '{}' not installed and Ollama has no local models.",
            requested
        );
    }

    let available = available_models.join(", ");
    format!(
        "Model '{}' not installed. Available models: {}",
        requested, available
    )
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_CODER_14B_MODEL, DEFAULT_CODER_14B_Q4KM_TAG, FALLBACK_PLANNER_MODEL,
        InstalledModelCard, OllamaClient, normalize_requested_model, resolve_model_name,
        should_enable_css_compression,
    };

    fn card(name: &str, quantization_level: Option<&str>) -> InstalledModelCard {
        InstalledModelCard {
            name: name.to_string(),
            parameter_size: None,
            quantization_level: quantization_level.map(ToOwned::to_owned),
        }
    }

    #[test]
    fn resolves_exact_installed_model_name() {
        let installed = vec![
            card(FALLBACK_PLANNER_MODEL, None),
            card("llama3.1:latest", None),
        ];

        assert_eq!(
            resolve_model_name("qwen3.5:latest", &installed).as_deref(),
            Some(FALLBACK_PLANNER_MODEL)
        );
    }

    #[test]
    fn resolves_legacy_qwen3_request_to_installed_qwen35_model() {
        let installed = vec![
            card(FALLBACK_PLANNER_MODEL, None),
            card("qwen2.5-coder:latest", None),
        ];

        assert_eq!(
            resolve_model_name("qwen3:8b", &installed).as_deref(),
            Some(FALLBACK_PLANNER_MODEL)
        );
    }

    #[test]
    fn normalizes_coder14b_alias() {
        assert_eq!(normalize_requested_model("14b"), DEFAULT_CODER_14B_MODEL);
        assert_eq!(
            normalize_requested_model("coder14b"),
            DEFAULT_CODER_14B_MODEL
        );
    }

    #[test]
    fn resolves_quantized_alias_to_matching_base_quantization() {
        let installed = vec![card(DEFAULT_CODER_14B_MODEL, Some("Q4_K_M"))];
        assert_eq!(
            resolve_model_name(DEFAULT_CODER_14B_Q4KM_TAG, &installed).as_deref(),
            Some(DEFAULT_CODER_14B_MODEL)
        );
    }

    #[test]
    fn enables_css_for_14b_models() {
        assert!(should_enable_css_compression("coder14b"));
        assert!(should_enable_css_compression(DEFAULT_CODER_14B_Q4KM_TAG));
        assert!(!should_enable_css_compression(FALLBACK_PLANNER_MODEL));
    }

    #[test]
    fn ollama_client_accepts_loopback_only() {
        let client = OllamaClient::local_default();
        assert_eq!(client.endpoint(), "http://127.0.0.1:11434");
        assert!(client.security_posture().contains("Loopback-only"));
    }

    #[test]
    #[should_panic(expected = "loopback-only")]
    fn ollama_client_rejects_remote_endpoint() {
        let _ = OllamaClient::new("http://192.168.1.10:11434".to_string());
    }
}
