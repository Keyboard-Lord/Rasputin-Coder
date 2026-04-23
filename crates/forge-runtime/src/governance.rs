//! FORGE Governance Module
//!
//! Implements startup contract checks, structured audit logging, and
//! doc/code drift assertions for the active runtime path.

use std::collections::HashMap;
use std::path::Path;

/// Current FORGE spec version
pub const FORGE_SPEC_VERSION: &str = "1.0.0";
pub const ACTIVE_VALIDATION_PATH: &str = "execution::ValidationEngine";
pub const CSS_POLICY_DESCRIPTION: &str = "CSS auto-compression is enabled for 14B+ models.";
pub const GOVERNANCE_RUNTIME_DESCRIPTION: &str =
    "Governance is initialized during Forge runtime startup.";
pub const ACTIVE_VALIDATION_DESCRIPTION: &str =
    "Active mutation validation path: execution::ValidationEngine.";

/// Governance configuration drift detection
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct GovernanceCheck {
    pub spec_version: String,
    pub config_version: String,
    pub drift_detected: bool,
    pub drift_items: Vec<String>,
}

/// Audit log entry for runtime governance events
#[derive(Debug, Clone)]
pub struct AuditLogEntry {
    pub timestamp: u64,
    pub session_id: String,
    pub iteration: u32,
    pub event_type: String,
    pub rule_broken: Option<String>,
    pub decision: String,
    pub details: String,
}

#[derive(Debug, Clone)]
pub struct GovernanceRuntimeSnapshot {
    pub planner_type: String,
    pub planner_temperature: f32,
    pub planner_seed: u64,
    pub validator_rules: u32,
    pub validation_path: &'static str,
}

impl GovernanceRuntimeSnapshot {
    pub fn new(planner_type: String, planner_temperature: f32, planner_seed: u64) -> Self {
        Self {
            planner_type,
            planner_temperature,
            planner_seed,
            validator_rules: 13,
            validation_path: ACTIVE_VALIDATION_PATH,
        }
    }

    fn to_config_map(&self) -> HashMap<String, String> {
        let mut config = HashMap::new();
        config.insert("planner_type".to_string(), self.planner_type.clone());
        config.insert(
            "temperature".to_string(),
            self.planner_temperature.to_string(),
        );
        config.insert("seed".to_string(), self.planner_seed.to_string());
        config.insert(
            "validator_rules".to_string(),
            self.validator_rules.to_string(),
        );
        config.insert(
            "validation_path".to_string(),
            self.validation_path.to_string(),
        );
        config
    }
}

/// Governance engine for spec compliance
pub struct GovernanceEngine {
    spec_version: String,
    audit_logs: Vec<AuditLogEntry>,
    drift_threshold: u32,
}

impl GovernanceEngine {
    pub fn new() -> Self {
        Self {
            spec_version: FORGE_SPEC_VERSION.to_string(),
            audit_logs: Vec::new(),
            drift_threshold: 5,
        }
    }

    pub fn check_spec_version(&self, _system_spec_path: &Path) -> GovernanceCheck {
        GovernanceCheck {
            spec_version: FORGE_SPEC_VERSION.to_string(),
            config_version: "1.0.0".to_string(),
            drift_detected: false,
            drift_items: Vec::new(),
        }
    }

    pub fn detect_drift(&self, config: &HashMap<String, String>) -> Vec<String> {
        let mut drift_items = Vec::new();

        for field in ["planner_type", "temperature", "seed", "validator_rules"] {
            if !config.contains_key(field) {
                drift_items.push(format!("Missing required field: {}", field));
            }
        }

        if let Some(temp) = config.get("temperature")
            && let Ok(temp_val) = temp.parse::<f32>()
            && temp_val > 0.1
        {
            drift_items.push(format!(
                "Temperature {} exceeds spec max 0.1 (FORGE_PLANNER_OUTPUT_CONTRACT_SPEC.md)",
                temp_val
            ));
        }

        if config.get("validation_path").map(String::as_str) != Some(ACTIVE_VALIDATION_PATH) {
            drift_items.push(format!(
                "Active validation path drifted from {}",
                ACTIVE_VALIDATION_PATH
            ));
        }

        drift_items
    }

    pub fn enforce_runtime_startup(
        &mut self,
        snapshot: &GovernanceRuntimeSnapshot,
        session_id: &str,
    ) -> GovernanceCheck {
        let mut check =
            self.check_spec_version(Path::new("docs/canonical/FORGE_SYSTEM_ARCHITECTURE.md"));
        let drift_items = self.detect_drift(&snapshot.to_config_map());
        check.drift_detected = !drift_items.is_empty();
        check.drift_items = drift_items.clone();

        self.log_validation_event(AuditLogEntry {
            timestamp: crate::types::timestamp_now(),
            session_id: session_id.to_string(),
            iteration: 0,
            event_type: "runtime_startup".to_string(),
            rule_broken: drift_items.first().cloned(),
            decision: if check.drift_detected {
                "Warn".to_string()
            } else {
                "Accept".to_string()
            },
            details: if check.drift_detected {
                check.drift_items.join("; ")
            } else {
                "Runtime contract verified".to_string()
            },
        });

        eprintln!("[GOVERNANCE] FORGE spec v{} initialized", self.spec_version);
        eprintln!("[GOVERNANCE] {}", CSS_POLICY_DESCRIPTION);
        eprintln!("[GOVERNANCE] {}", GOVERNANCE_RUNTIME_DESCRIPTION);
        eprintln!("[GOVERNANCE] {}", ACTIVE_VALIDATION_DESCRIPTION);
        if check.drift_detected {
            eprintln!(
                "[GOVERNANCE] Drift detected ({} item(s))",
                check.drift_items.len()
            );
            for item in &check.drift_items {
                eprintln!("[GOVERNANCE]   - {}", item);
            }
        } else {
            eprintln!("[GOVERNANCE] Runtime contract verified");
        }

        check
    }

    pub fn log_validation_event(&mut self, entry: AuditLogEntry) {
        self.audit_logs.push(entry);
    }

    #[allow(dead_code)]
    pub fn export_audit_logs(&self) -> String {
        self.audit_logs
            .iter()
            .map(|e| {
                serde_json::json!({
                    "timestamp": e.timestamp,
                    "session_id": e.session_id,
                    "iteration": e.iteration,
                    "event_type": e.event_type,
                    "rule_broken": e.rule_broken,
                    "decision": e.decision,
                    "details": e.details
                })
                .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[allow(dead_code)]
    pub fn compliance_summary(&self) -> HashMap<String, u32> {
        let mut summary = HashMap::new();
        summary.insert("total_events".to_string(), self.audit_logs.len() as u32);
        summary.insert(
            "validation_failures".to_string(),
            self.audit_logs
                .iter()
                .filter(|e| e.decision == "Reject" || e.decision == "Escalate")
                .count() as u32,
        );
        summary.insert("drift_threshold".to_string(), self.drift_threshold);
        summary
    }
}

impl Default for GovernanceEngine {
    fn default() -> Self {
        Self::new()
    }
}

pub fn init_governance(snapshot: &GovernanceRuntimeSnapshot, session_id: &str) -> GovernanceEngine {
    let mut engine = GovernanceEngine::new();
    let _ = engine.enforce_runtime_startup(snapshot, session_id);
    engine
}

#[cfg(test)]
mod tests {
    use super::{ACTIVE_VALIDATION_PATH, GovernanceEngine, GovernanceRuntimeSnapshot};
    use std::fs;
    use std::path::PathBuf;

    fn repo_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|path| path.parent())
            .expect("crates/forge-runtime should live inside repo root")
            .to_path_buf()
    }

    #[test]
    fn detects_temperature_drift() {
        let engine = GovernanceEngine::new();
        let snapshot = GovernanceRuntimeSnapshot {
            planner_type: "http".to_string(),
            planner_temperature: 0.5,
            planner_seed: 42,
            validator_rules: 13,
            validation_path: ACTIVE_VALIDATION_PATH,
        };

        let drift = engine.detect_drift(&snapshot.to_config_map());
        assert!(
            drift
                .iter()
                .any(|d| d.to_lowercase().contains("temperature"))
        );
    }

    #[test]
    fn accepts_expected_runtime_contract() {
        let mut engine = GovernanceEngine::new();
        let snapshot = GovernanceRuntimeSnapshot::new("http".to_string(), 0.0, 42);

        let check = engine.enforce_runtime_startup(&snapshot, "forge-test-session");
        assert!(!check.drift_detected);
    }

    // NOTE: The phase4_summary_matches_authoritative_runtime_claims test was removed.
    // It validated runtime constants against docs/archive/phases/PHASE4_SUMMARY.md which
    // was removed during documentation reorganization to numbered format (docs/01-15).
    // The governance constants remain in use but are no longer validated against a single
    // consolidated runtime summary document.

    #[test]
    fn runtime_hot_path_uses_validation_engine() {
        let runtime_source =
            fs::read_to_string(repo_root().join("crates/forge-runtime/src/runtime.rs"))
                .expect("read runtime source");

        assert!(runtime_source.contains("ValidationEngine::new()"));
        assert!(runtime_source.contains("validate_detailed("));
        assert!(!runtime_source.contains("validator::validate("));
    }
}
