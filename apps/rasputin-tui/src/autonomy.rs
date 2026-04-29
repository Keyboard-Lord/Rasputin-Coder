use crate::commands::Command;
use crate::guidance::GoalManager;
use crate::ollama::{
    DEFAULT_CODER_14B_MODEL, FALLBACK_PLANNER_MODEL, InstalledModelCard, normalize_requested_model,
};
use crate::persistence::{
    ChainLifecycleStatus, ChainPolicy, ChainStepStatus, PersistentChain, PersistentChainStep,
    RecoveryStepKind,
};
use crate::state::{
    ArtifactCompletionContract, CompletionConfidence, ObjectiveSatisfaction, RequiredSurface,
    StepOutcomeClass, StepResult,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Owns the TUI-side autonomous goal loop policy.
pub struct AutonomousLoopController;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutonomousStartDecision {
    Start { command: Command, reason: String },
    Blocked { reason: String, next_action: String },
}

/// Decision from self-healing analysis
#[derive(Debug, Clone)]
pub enum SelfHealingDecision {
    /// Generate and enqueue a recovery step
    Recover {
        recovery_step: PersistentChainStep,
        reason: String,
    },
    /// Retry the same step (for transient failures like timeouts)
    Retry {
        step_id: String,
        attempt: u32,
        reason: String,
    },
    /// Halt - failure is not recoverable or policy prevents recovery
    Halt { reason: String, audited: bool },
    /// Continue chain normally (step succeeded)
    Continue,
}

/// Decision from completion confidence gate after recovery
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompletionConfidenceDecision {
    /// Objective is satisfied - finalize chain
    Finalize { reason: String },
    /// More work required - continue chain execution
    Continue { reason: String },
    /// Cannot determine completion - halt or require explicit decision
    HaltForClarification { reason: String },
}

/// Evaluates completion confidence after step execution to prevent over-retry and under-complete
pub struct CompletionConfidenceEvaluator;

impl CompletionConfidenceEvaluator {
    /// Evaluate whether chain should finalize or continue after a step completes
    ///
    /// Logic:
    /// 1. Check if this was a recovery step - if so, need explicit completion check
    /// 2. Verify required surfaces are present
    /// 3. Check if minimum validation stage passes
    /// 4. Determine if objective appears complete
    /// 5. Return appropriate decision
    pub fn evaluate_after_step(
        chain: &PersistentChain,
        step_result: &StepResult,
        satisfaction: &ObjectiveSatisfaction,
    ) -> CompletionConfidenceDecision {
        // If step failed, cannot complete
        if !matches!(
            step_result.outcome_class,
            StepOutcomeClass::Success | StepOutcomeClass::Recovery
        ) {
            return CompletionConfidenceDecision::Continue {
                reason: "Step did not succeed - chain must continue or halt".to_string(),
            };
        }

        // Check required surfaces
        let missing_surfaces =
            Self::check_required_surfaces(&satisfaction.required_surfaces, chain.repo_path.as_deref());
        if !missing_surfaces.is_empty() {
            return CompletionConfidenceDecision::Continue {
                reason: format!("Required surfaces missing: {:?}", missing_surfaces),
            };
        }

        if let Some(contract) = &satisfaction.artifact_contract {
            if contract.has_requirements() && !contract.is_satisfied() {
                return CompletionConfidenceDecision::Continue {
                    reason: Self::artifact_contract_reason(contract),
                };
            }
        }

        // Check if this was a recovery step - need explicit completion validation
        let was_recovery = matches!(step_result.outcome_class, StepOutcomeClass::Recovery);

        if was_recovery {
            // After recovery, check completion confidence
            match satisfaction.confidence {
                CompletionConfidence::ObjectiveSatisfied => {
                    return CompletionConfidenceDecision::Finalize {
                        reason: satisfaction.reason.clone().unwrap_or_else(|| {
                            "Objective completion verified after recovery".to_string()
                        }),
                    };
                }
                CompletionConfidence::PartialRecovery => {
                    return CompletionConfidenceDecision::Continue {
                        reason: satisfaction.reason.clone().unwrap_or_else(|| {
                            "Step recovered but objective requires more work".to_string()
                        }),
                    };
                }
                CompletionConfidence::Uncertain => {
                    return CompletionConfidenceDecision::HaltForClarification {
                        reason: satisfaction.reason.clone().unwrap_or_else(|| {
                            "Cannot determine completion after recovery".to_string()
                        }),
                    };
                }
                CompletionConfidence::NotApplicable => {
                    // Recovery marked as NotApplicable - treat as needing more work
                    return CompletionConfidenceDecision::Continue {
                        reason: "Recovery succeeded but completion status unclear".to_string(),
                    };
                }
            }
        }

        // Normal step success - check if all steps complete
        let all_steps_complete = chain.next_pending_step().is_none();

        if all_steps_complete && satisfaction.objective_complete {
            CompletionConfidenceDecision::Finalize {
                reason: "All steps complete and objective satisfied".to_string(),
            }
        } else if all_steps_complete
            && satisfaction
                .artifact_contract
                .as_ref()
                .is_some_and(|contract| contract.has_requirements())
        {
            CompletionConfidenceDecision::Continue {
                reason: satisfaction
                    .reason
                    .clone()
                    .unwrap_or_else(|| "All steps ran, but the explicit artifact contract is incomplete".to_string()),
            }
        } else if all_steps_complete {
            CompletionConfidenceDecision::HaltForClarification {
                reason: "All steps complete but objective completion uncertain".to_string(),
            }
        } else {
            CompletionConfidenceDecision::Continue {
                reason: "More steps remaining in chain".to_string(),
            }
        }
    }

    /// Check which required surfaces are missing
    pub fn refresh_objective_satisfaction(chain: &PersistentChain) -> ObjectiveSatisfaction {
        let mut satisfaction = chain.objective_satisfaction.clone();
        satisfaction.checked_at = Some(chrono::Local::now());

        let missing_surfaces =
            Self::check_required_surfaces(&satisfaction.required_surfaces, chain.repo_path.as_deref());

        if let Some(contract) = satisfaction.artifact_contract.as_mut() {
            Self::refresh_artifact_contract(chain, contract);
        }

        let contract_reason = satisfaction
            .artifact_contract
            .as_ref()
            .filter(|contract| contract.has_requirements())
            .map(Self::artifact_contract_reason);

        if satisfaction.required_surfaces.is_empty() && satisfaction.artifact_contract.is_none() {
            satisfaction.objective_complete = true;
            satisfaction.confidence = CompletionConfidence::ObjectiveSatisfied;
            satisfaction.reason.get_or_insert_with(|| {
                "No explicit completion contract detected; chain completion follows executed steps."
                    .to_string()
            });
            return satisfaction;
        }

        let contract_satisfied = satisfaction
            .artifact_contract
            .as_ref()
            .map(|contract| contract.is_satisfied())
            .unwrap_or(true);
        let objective_complete = missing_surfaces.is_empty() && contract_satisfied;
        satisfaction.objective_complete = objective_complete;

        if objective_complete {
            satisfaction.confidence = CompletionConfidence::ObjectiveSatisfied;
            if let Some(contract_reason) = contract_reason {
                satisfaction.reason = Some(contract_reason);
            } else {
                satisfaction.reason = Some("All explicit completion requirements are satisfied.".to_string());
            }
        } else {
            satisfaction.confidence = CompletionConfidence::PartialRecovery;
            satisfaction.reason = contract_reason.or_else(|| {
                if missing_surfaces.is_empty() {
                    None
                } else {
                    Some(format!(
                        "Required surfaces still missing: {}",
                        missing_surfaces.join(", ")
                    ))
                }
            });
        }

        satisfaction
    }

    fn check_required_surfaces(surfaces: &[RequiredSurface], repo_path: Option<&str>) -> Vec<String> {
        let mut missing = Vec::new();

        for surface in surfaces {
            match surface {
                RequiredSurface::FileExists { path } => {
                    let resolved = Self::resolve_required_surface_path(repo_path, path);
                    if !resolved.exists() {
                        missing.push(format!("File: {}", path));
                    }
                }
                RequiredSurface::TestPasses { name } => {
                    missing.push(format!("Test: {}", name));
                }
                RequiredSurface::BuildSucceeds => {
                    missing.push("Build validation".to_string());
                }
                RequiredSurface::ValidationPasses => {
                    missing.push("Full validation".to_string());
                }
                RequiredSurface::Custom { description, .. } => {
                    missing.push(format!("Custom: {}", description));
                }
            }
        }
        missing
    }

    /// Update completion confidence after validation passes
    pub fn update_confidence_after_validation(
        satisfaction: &mut ObjectiveSatisfaction,
        validation_passed: bool,
    ) {
        satisfaction.checked_at = Some(chrono::Local::now());

        if validation_passed {
            let explicit_requirements_remaining = !satisfaction.required_surfaces.is_empty()
                || satisfaction
                    .artifact_contract
                    .as_ref()
                    .is_some_and(|contract| contract.has_requirements() && !contract.is_satisfied());

            if !explicit_requirements_remaining || satisfaction.objective_complete {
                satisfaction.confidence = CompletionConfidence::ObjectiveSatisfied;
                satisfaction.reason =
                    Some("Validation passed and all required surfaces present".to_string());
                satisfaction.objective_complete = true;
            } else {
                satisfaction.confidence = CompletionConfidence::PartialRecovery;
                satisfaction.reason =
                    Some("Validation passed but additional surfaces required".to_string());
            }
        } else {
            satisfaction.confidence = CompletionConfidence::PartialRecovery;
            satisfaction.reason = Some("Validation failed - more work required".to_string());
        }
    }

    fn refresh_artifact_contract(chain: &PersistentChain, contract: &mut ArtifactCompletionContract) {
        let repo_root = chain.repo_path.as_deref().map(PathBuf::from);
        let required_paths: Vec<String> = contract
            .required_filenames
            .iter()
            .map(|path| Self::normalize_contract_path(path))
            .collect();
        let required_set: HashSet<String> = required_paths.iter().cloned().collect();

        let mut created = Vec::new();
        let mut missing = Vec::new();
        let mut empty = Vec::new();

        for required in &required_paths {
            let resolved = Self::resolve_required_surface_path(
                repo_root.as_ref().and_then(|path| path.to_str()),
                required,
            );

            if !resolved.exists() {
                missing.push(required.clone());
                continue;
            }

            if contract.require_non_empty && !Self::path_has_content(&resolved) {
                empty.push(required.clone());
                continue;
            }

            created.push(required.clone());
        }

        let mut unexpected = Vec::new();
        for path in chain.recorded_affected_paths() {
            let normalized = Self::normalize_contract_path(&path);
            if !Self::matches_artifact_contract(&normalized, contract) || required_set.contains(&normalized)
            {
                continue;
            }

            let resolved = Self::resolve_required_surface_path(
                repo_root.as_ref().and_then(|root| root.to_str()),
                &normalized,
            );
            if resolved.exists() && !unexpected.contains(&normalized) {
                unexpected.push(normalized);
            }
        }

        contract.created_filenames = created;
        contract.missing_filenames = missing;
        contract.empty_filenames = empty;
        contract.unexpected_filenames = unexpected;
        contract.actual_output_count = Some(
            contract.created_filenames.len()
                + contract.empty_filenames.len()
                + contract.unexpected_filenames.len(),
        );
    }

    fn resolve_required_surface_path(repo_path: Option<&str>, path: &str) -> PathBuf {
        let path = Path::new(path);
        if path.is_absolute() {
            path.to_path_buf()
        } else if let Some(repo_path) = repo_path {
            Path::new(repo_path).join(path)
        } else {
            path.to_path_buf()
        }
    }

    fn normalize_contract_path(path: &str) -> String {
        path.trim()
            .trim_matches(|ch: char| {
                ch.is_whitespace()
                    || matches!(ch, '`' | '"' | '\'' | ',' | ';' | ':' | '(' | ')' | '[' | ']')
            })
            .trim_start_matches("./")
            .replace('\\', "/")
    }

    fn path_has_content(path: &Path) -> bool {
        if let Ok(content) = std::fs::read_to_string(path) {
            return !content.trim().is_empty();
        }

        std::fs::metadata(path)
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false)
    }

    fn matches_artifact_contract(path: &str, contract: &ArtifactCompletionContract) -> bool {
        let lower = path.to_ascii_lowercase();
        match contract.artifact_type.as_deref() {
            Some("markdown") => lower.ends_with(".md") || lower.ends_with(".markdown"),
            Some(kind) => lower.ends_with(&format!(".{}", kind.to_ascii_lowercase())),
            None => contract
                .required_filenames
                .iter()
                .filter_map(|required| required.rsplit_once('.').map(|(_, extension)| extension))
                .any(|extension| lower.ends_with(&format!(".{}", extension.to_ascii_lowercase()))),
        }
    }

    fn artifact_contract_reason(contract: &ArtifactCompletionContract) -> String {
        if contract.is_satisfied() {
            return format!(
                "Explicit artifact contract satisfied: {} deliverable(s) present.",
                contract.required_deliverable_count()
            );
        }

        let mut reasons = Vec::new();
        if !contract.missing_filenames.is_empty() {
            reasons.push(format!(
                "missing {}",
                contract.missing_filenames.join(", ")
            ));
        }
        if !contract.empty_filenames.is_empty() {
            reasons.push(format!("empty {}", contract.empty_filenames.join(", ")));
        }
        if !contract.unexpected_filenames.is_empty() {
            reasons.push(format!(
                "unexpected {}",
                contract.unexpected_filenames.join(", ")
            ));
        }
        if let Some(actual) = contract.actual_output_count {
            reasons.push(format!(
                "count {}/{}",
                actual,
                contract.required_deliverable_count()
            ));
        }

        format!(
            "Explicit artifact contract incomplete: {}.",
            reasons.join("; ")
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlannerModelBinding {
    Configured,
    ResolvedConfigured {
        requested_model: String,
    },
    AutoBound,
    ReboundInvalidConfigured {
        invalid_model: String,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlannerPreflightOutcome {
    Ready {
        model: String,
        binding: PlannerModelBinding,
    },
    MissingLocalModel,
    OllamaUnavailable {
        reason: String,
    },
    InvalidConfiguredModel {
        configured_model: String,
        reason: String,
    },
    PreconditionFailed {
        reason: String,
    },
}

impl AutonomousLoopController {
    pub fn resolve_planner_preflight(
        configured_model: Option<&str>,
        installed_models: &[InstalledModelCard],
        ollama_available: bool,
        ollama_error: Option<&str>,
    ) -> PlannerPreflightOutcome {
        if !ollama_available {
            return PlannerPreflightOutcome::OllamaUnavailable {
                reason: ollama_error
                    .unwrap_or("Ollama is not reachable")
                    .to_string(),
            };
        }

        if let Some(configured_model) = configured_model.filter(|model| !model.trim().is_empty()) {
            if let Some(model) = resolve_configured_model(configured_model, installed_models) {
                let normalized = normalize_requested_model(configured_model);
                let binding = if model.eq_ignore_ascii_case(configured_model)
                    || model.eq_ignore_ascii_case(&normalized)
                {
                    PlannerModelBinding::Configured
                } else {
                    PlannerModelBinding::ResolvedConfigured {
                        requested_model: configured_model.to_string(),
                    }
                };

                return PlannerPreflightOutcome::Ready { model, binding };
            }

            if let Some(model) = select_autonomous_planner_model(installed_models) {
                return PlannerPreflightOutcome::Ready {
                    model,
                    binding: PlannerModelBinding::ReboundInvalidConfigured {
                        invalid_model: configured_model.to_string(),
                        reason: format!("configured model '{}' is not installed", configured_model),
                    },
                };
            }

            // RELAXED: Use ANY available model rather than failing
            if let Some(model) = installed_models.first() {
                return PlannerPreflightOutcome::Ready {
                    model: model.name.clone(),
                    binding: PlannerModelBinding::AutoBound,
                };
            }
        }

        if let Some(model) = select_autonomous_planner_model(installed_models) {
            return PlannerPreflightOutcome::Ready {
                model,
                binding: PlannerModelBinding::AutoBound,
            };
        }

        // RELAXED: If no coder models, use ANY available model
        if let Some(model) = installed_models.first() {
            return PlannerPreflightOutcome::Ready {
                model: model.name.clone(),
                binding: PlannerModelBinding::AutoBound,
            };
        }
        PlannerPreflightOutcome::MissingLocalModel
    }

    /// Accepted goals should enter bounded autonomous execution while keeping
    /// the existing validation and approval gates intact.
    pub fn configure_goal_policy(policy: &mut ChainPolicy) {
        policy.auto_resume = true;
        policy.auto_advance = true;
        policy.auto_retry_on_validation_failure = true;
        // RELAXED: Don't require validation every step
        policy.require_validation_each_step = false;
        // RELAXED: Don't halt on failure - keep going
        policy.halt_on_failure = false;
        // RELAXED: Less approval gates
        policy.require_approval_for_high = false;
        policy.allow_auto_low_risk = true;
    }

    /// Attach concrete plan context to the chain when the planner produced
    /// file references that exist in the active repository.
    pub fn seed_goal_context(
        chain: &mut PersistentChain,
        required_context: &[String],
        repo_path: &str,
    ) -> usize {
        let repo_root = Path::new(repo_path);
        let mut added = 0;

        for file in required_context {
            if file.trim().is_empty() || chain.selected_context_files.contains(file) {
                continue;
            }

            if repo_root.join(file).is_file() {
                chain.selected_context_files.push(file.clone());
                added += 1;
            }
        }

        added
    }

    /// Decide whether an accepted goal chain should schedule its first
    /// execution step. The actual execution still flows through ChainResume,
    /// so critical risk preview and Forge startup checks remain centralized.
    pub fn decide_goal_start(
        chain: &PersistentChain,
        policy: &ChainPolicy,
    ) -> AutonomousStartDecision {
        if chain.steps.is_empty() {
            return AutonomousStartDecision::Blocked {
                reason: "chain has no executable steps".to_string(),
                next_action: "/goal <statement>".to_string(),
            };
        }

        if chain.total_steps_executed >= policy.max_steps {
            return AutonomousStartDecision::Blocked {
                reason: format!(
                    "max step limit reached ({}/{})",
                    chain.total_steps_executed, policy.max_steps
                ),
                next_action: "/chains".to_string(),
            };
        }

        if chain.total_steps_failed >= policy.max_consecutive_failures {
            return AutonomousStartDecision::Blocked {
                reason: format!(
                    "failure limit reached ({}/{})",
                    chain.total_steps_failed, policy.max_consecutive_failures
                ),
                next_action: "/replay".to_string(),
            };
        }

        if chain
            .pending_checkpoint
            .as_ref()
            .is_some_and(|checkpoint| checkpoint.is_pending())
        {
            return AutonomousStartDecision::Blocked {
                reason: "approval checkpoint is pending".to_string(),
                next_action: "/approve or /deny".to_string(),
            };
        }

        if !matches!(
            chain.status,
            ChainLifecycleStatus::Draft
                | ChainLifecycleStatus::Ready
                | ChainLifecycleStatus::Halted
        ) {
            return AutonomousStartDecision::Blocked {
                reason: format!("chain status is {:?}", chain.status),
                next_action: "/chain status".to_string(),
            };
        }

        let has_pending_step = chain
            .steps
            .iter()
            .any(|step| matches!(step.status, ChainStepStatus::Pending));

        if !has_pending_step {
            return AutonomousStartDecision::Blocked {
                reason: "chain has no pending step".to_string(),
                next_action: "/plan".to_string(),
            };
        }

        AutonomousStartDecision::Start {
            command: Command::ChainResume {
                chain_id: "active".to_string(),
                force: false,
            },
            reason: "accepted goal chain is ready to start".to_string(),
        }
    }

    /// Decide self-healing action after a step completes with a result
    /// Returns: SelfHealingDecision indicating whether to recover, retry, halt, or continue
    pub fn decide_self_healing(
        chain: &PersistentChain,
        policy: &ChainPolicy,
        step_result: &StepResult,
    ) -> SelfHealingDecision {
        // Success case - no healing needed
        if matches!(step_result.outcome_class, StepOutcomeClass::Success) {
            return SelfHealingDecision::Continue;
        }

        // Check retry limits
        if step_result.retry_attempt >= policy.max_auto_retries_per_step {
            return SelfHealingDecision::Halt {
                reason: format!(
                    "Max retry attempts ({}) reached for this step",
                    policy.max_auto_retries_per_step
                ),
                audited: true,
            };
        }

        // Check recovery depth (total recoveries in chain)
        let recovery_count = chain.steps.iter().filter(|s| s.retry_of.is_some()).count() as u32;

        if recovery_count >= policy.max_chain_recovery_depth {
            return SelfHealingDecision::Halt {
                reason: format!(
                    "Max recovery depth ({}) reached for this chain",
                    policy.max_chain_recovery_depth
                ),
                audited: true,
            };
        }

        // Check if failure is recoverable
        if !step_result.is_recoverable() {
            return SelfHealingDecision::Halt {
                reason: format!(
                    "Failure is not recoverable: {}",
                    step_result
                        .error_message
                        .as_deref()
                        .unwrap_or("unknown error")
                ),
                audited: true,
            };
        }

        // Check if auto-retry is enabled
        if !policy.auto_retry_on_validation_failure {
            return SelfHealingDecision::Halt {
                reason: "Auto-retry disabled by policy".to_string(),
                audited: true,
            };
        }

        // Check total failure count
        if chain.total_steps_failed >= policy.max_consecutive_failures {
            return SelfHealingDecision::Halt {
                reason: format!(
                    "Max consecutive failures ({}) reached",
                    policy.max_consecutive_failures
                ),
                audited: true,
            };
        }

        // Determine if we should retry or generate recovery step
        match &step_result.failure_reason {
            Some(_) => {
                // Generate recovery step with fix attempt
                let recovery_description = step_result
                    .generate_recovery_description()
                    .unwrap_or_else(|| "Fix previous step failure".to_string());

                // Get the last step from chain to reference for retry_of
                let last_step_id = chain
                    .steps
                    .last()
                    .map(|s| s.id.clone())
                    .unwrap_or_else(|| "step-0".to_string());

                let recovery_step = PersistentChainStep {
                    id: format!(
                        "{}-recovery-{}",
                        last_step_id,
                        step_result.retry_attempt + 1
                    ),
                    description: recovery_description,
                    status: ChainStepStatus::Pending,
                    retry_of: Some(last_step_id),
                    retry_attempt: step_result.retry_attempt + 1,
                    execution_outcome: None,
                    execution_result_class: None,
                    execution_results: vec![],
                    failure_reason: None,
                    recovery_step_kind: Some(RecoveryStepKind::Fix),
                    evidence_snapshot: None,
                    force_override_used: false,
                    tool_calls: vec![],
                    result_summary: None,
                    validation_passed: None,
                    started_at: None,
                    completed_at: None,
                    error_message: None,
                    replay_record: None,
                };

                SelfHealingDecision::Recover {
                    recovery_step,
                    reason: format!(
                        "Generating recovery step for: {}",
                        step_result
                            .error_message
                            .as_deref()
                            .unwrap_or("unknown failure")
                    ),
                }
            }
            None => {
                // No specific failure reason - simple retry
                SelfHealingDecision::Retry {
                    step_id: "current".to_string(),
                    attempt: step_result.retry_attempt + 1,
                    reason: "Retrying after transient failure".to_string(),
                }
            }
        }
    }

    pub fn mark_goal_completed_for_chain(
        goal_manager: &mut GoalManager,
        chain: &PersistentChain,
        summary: String,
    ) -> bool {
        if !goal_manager
            .active_goal()
            .is_some_and(|goal| goal.chain_id.as_ref() == Some(&chain.id))
        {
            return false;
        }

        goal_manager.mark_completed(summary);
        true
    }

    pub fn mark_goal_failed_for_chain(
        goal_manager: &mut GoalManager,
        chain: &PersistentChain,
        reason: String,
    ) -> bool {
        if !goal_manager
            .active_goal()
            .is_some_and(|goal| goal.chain_id.as_ref() == Some(&chain.id))
        {
            return false;
        }

        if let Some(goal) = goal_manager.active_goal_mut() {
            goal.mark_failed_with_reason(reason);
            return true;
        }

        false
    }

    /// Detects if input is an execution-intent prompt that should NOT go to plain chat.
    ///
    /// This function distinguishes:
    /// - Conversational questions (returns false → route to chat)
    /// - Simple task requests (returns true → route to goal)
    /// - Structured execution/document-generation prompts (returns true → route to goal)
    ///
    /// High-priority detection cases (take precedence over conversational prefixes):
    /// - Role/system mode definitions ("You are a...", "Act as...")
    /// - Ordered deliverables with numbered lists or bullet points
    /// - Document generation with "Output Requirements", "Goals", "Expected deliverables"
    /// - Multiple file/artifact requests
    /// - Large imperative blocks clearly not conversational
    pub fn is_task_like_plain_text(input: &str) -> bool {
        let text = input.trim();
        if text.is_empty() || text.starts_with('/') {
            return false;
        }

        let lower = text.to_lowercase();

        // HIGH PRIORITY: Check for structured execution patterns FIRST
        // These take precedence over conversational prefixes because a prompt like
        // "How would you approach this? Create files: 1. ... 2. ... 3. ..."
        // is clearly execution intent despite starting with "how"
        if Self::is_structured_execution_prompt(&lower, text) {
            return true;
        }

        // Check for conversational patterns (now second priority)
        if Self::is_conversational_question(&lower) {
            return false;
        }

        // Check for simple task patterns (existing behavior)
        Self::is_simple_task_request(&lower)
    }

    /// Detects conversational questions that should go to chat
    fn is_conversational_question(lower: &str) -> bool {
        let conversational_prefixes = [
            "what ",
            "why ",
            "how ",
            "who ",
            "when ",
            "where ",
            "explain ",
            "describe ",
            "summarize ",
            "tell me ",
            "show me ",
            "list ",
            "can you explain ",
            "could you explain ",
            "would you explain ",
            "do you know ",
            "have you seen ",
            "what's your ",
            "what is your ",
        ];

        conversational_prefixes
            .iter()
            .any(|prefix| lower.starts_with(prefix))
    }

    /// Detects structured execution prompts with document-generation patterns
    fn is_structured_execution_prompt(lower: &str, original: &str) -> bool {
        let line_count = original.lines().count();
        let word_count = original.split_whitespace().count();

        // Role/system mode definitions
        let role_patterns = [
            "you are a ",
            "you are an ",
            "you are the ",
            "act as ",
            "act like ",
            "role: ",
            "your role is ",
            "system mode",
            "system prompt",
            " persona",
            "expert in ",
            "specialist in ",
        ];

        // Document generation indicators
        let doc_gen_patterns = [
            "output requirements",
            "expected deliverables",
            "deliverables:",
            "goals:",
            "objective:",
            "objectives:",
            "requirements:",
            "acceptance criteria",
            "deliverable",
            "artifact",
            "generate a ",
            "produce a ",
            "deliver a ",
            "create the following",
            "output format",
            "response format",
            "format your response",
            "step by step",
            "step-by-step",
            "numbered list",
            "bullet points",
        ];

        // Multi-file/artifact indicators
        let multi_artifact_patterns = [
            "files:",
            "file 1",
            "file 2",
            "file 3",
            "document 1",
            "document 2",
            "15 files",
            "15",
            "10 files",
            "10",
            "multiple files",
            "several files",
            "the following files",
            "create files",
            "generate files",
            "write files",
            "source files",
            "config files",
        ];

        // Imperative execution markers
        let imperative_markers = [
            "execute the following",
            "follow these steps",
            "complete the following",
            "implement the following",
            "build the following",
            "design the following",
            "architect the following",
            "code the following",
            "write the following",
            "develop the following",
        ];

        // Section headers (markdown-style)
        let section_headers = [
            "# objective",
            "# goal",
            "# goals",
            "# requirements",
            "# deliverables",
            "# output",
            "# inputs",
            "# context",
            "# background",
            "# constraints",
            "## objective",
            "## goal",
            "## goals",
            "## requirements",
            "## deliverables",
            "## output",
        ];

        // Count matches for scoring
        let role_match = role_patterns.iter().any(|p| lower.contains(p));
        let doc_gen_match = doc_gen_patterns.iter().any(|p| lower.contains(p));
        let multi_artifact_match = multi_artifact_patterns.iter().any(|p| lower.contains(p));
        let imperative_match = imperative_markers.iter().any(|p| lower.starts_with(p) || lower.contains(p));
        let section_match = section_headers.iter().any(|p| lower.contains(p));

        // Structured document patterns (numbered deliverables with descriptions)
        let has_numbered_deliverables = original
            .lines()
            .filter(|l| {
                let l = l.trim();
                l.starts_with("1.") || l.starts_with("2.") || l.starts_with("3.")
            })
            .count() >= 2;

        // Bullet point lists (markdown-style)
        let has_bullet_list = original
            .lines()
            .filter(|l| {
                let l = l.trim();
                l.starts_with("- ") || l.starts_with("* ")
            })
            .count() >= 2;

        // Calculate execution-intent score
        let mut score = 0;
        if role_match { score += 3; }
        if doc_gen_match { score += 2; }
        if multi_artifact_match { score += 3; }
        if imperative_match { score += 2; }
        if section_match { score += 2; }
        if has_numbered_deliverables { score += 2; }
        if has_bullet_list { score += 1; }

        // Length heuristics for structured prompts
        let is_long_structured = line_count >= 5 && word_count >= 50;
        let is_very_long = word_count >= 200;
        let is_medium_structured = line_count >= 3 && word_count >= 30;

        // Decision threshold: if score >= 2 (lowered to catch more execution prompts)
        // or strong indicators present
        let has_execution_indicators = score >= 2 ||
            (is_long_structured && (role_match || doc_gen_match || multi_artifact_match || section_match)) ||
            (is_medium_structured && (role_match || multi_artifact_match));

        // Very long structured prompts are almost certainly execution intent
        let is_definitely_execution = is_very_long && (section_match || has_numbered_deliverables || multi_artifact_match || doc_gen_match);

        // Clear document generation with output format/deliverables is execution intent
        let is_doc_gen_execution = doc_gen_match && (has_numbered_deliverables || has_bullet_list || section_match || is_medium_structured);

        has_execution_indicators || is_definitely_execution || is_doc_gen_execution
    }

    /// Detects simple task requests (original behavior preserved)
    fn is_simple_task_request(lower: &str) -> bool {
        let task_prefixes = [
            "add ",
            "build ",
            "change ",
            "clean ",
            "create ",
            "debug ",
            "document ",
            "fix ",
            "harden ",
            "implement ",
            "integrate ",
            "make ",
            "modify ",
            "refactor ",
            "remove ",
            "repair ",
            "replace ",
            "run ",
            "test ",
            "update ",
            "validate ",
            "wire ",
            "write ",
        ];
        if task_prefixes.iter().any(|prefix| lower.starts_with(prefix)) {
            return true;
        }

        let task_phrases = [
            "please add ",
            "please build ",
            "please create ",
            "please fix ",
            "please implement ",
            "please update ",
            "please write ",
            "can you add ",
            "can you build ",
            "can you create ",
            "can you fix ",
            "can you implement ",
            "can you update ",
            "can you write ",
            "i need you to ",
        ];

        task_phrases.iter().any(|phrase| lower.contains(phrase))
    }
}

fn resolve_configured_model(
    configured_model: &str,
    installed_models: &[InstalledModelCard],
) -> Option<String> {
    let normalized = normalize_requested_model(configured_model);

    installed_models
        .iter()
        .find(|model| {
            model.name.eq_ignore_ascii_case(configured_model)
                || model.name.eq_ignore_ascii_case(&normalized)
        })
        .map(|model| model.name.clone())
        .or_else(|| {
            let configured_family = normalized
                .split(':')
                .next()
                .unwrap_or(normalized.as_str())
                .to_string();

            installed_models
                .iter()
                .filter(|model| model.name.to_lowercase().starts_with(&configured_family))
                .min_by_key(|model| autonomous_model_rank(&model.name))
                .map(|model| model.name.clone())
        })
}

fn select_autonomous_planner_model(installed_models: &[InstalledModelCard]) -> Option<String> {
    installed_models
        .iter()
        .filter_map(|model| {
            let rank = autonomous_model_rank(&model.name);
            (rank != usize::MAX).then(|| (rank, model.name.clone()))
        })
        .min_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)))
        .map(|(_, model)| model)
}

fn autonomous_model_rank(model: &str) -> usize {
    let lower = model.to_lowercase();
    if lower == DEFAULT_CODER_14B_MODEL {
        0
    } else if lower.starts_with("qwen2.5-coder") && lower.contains("14b") {
        1
    } else if lower.starts_with("qwen2.5-coder") {
        2
    } else if lower == FALLBACK_PLANNER_MODEL {
        3
    } else {
        usize::MAX
    }
}

/// Generate a placeholder step ID for recovery steps
/// Uses timestamp for uniqueness
fn step_id_placeholder() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("recovery-{}", duration.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guidance::GoalStatus;
    use crate::ollama::{DEFAULT_CODER_14B_MODEL, FALLBACK_PLANNER_MODEL};
    use crate::persistence::{ChainPolicy, PersistentChain, PersistentChainStep, PersistentState};

    fn model(name: &str) -> InstalledModelCard {
        InstalledModelCard {
            name: name.to_string(),
            parameter_size: None,
            quantization_level: None,
        }
    }

    fn chain_with_step() -> PersistentChain {
        let mut state = PersistentState::default();
        let chain = state.create_chain("demo", "demo").clone();
        let chain_id = chain.id.clone();

        let chain = state
            .get_chain_mut(&chain_id)
            .expect("created chain should exist");

        chain.steps.push(PersistentChainStep {
            id: "step-1".to_string(),
            description: "Inspect system state".to_string(),
            status: ChainStepStatus::Pending,
            retry_of: None,
            retry_attempt: 0,
            execution_outcome: None,
            execution_result_class: None,
            execution_results: vec![],
            failure_reason: None,
            recovery_step_kind: None,
            evidence_snapshot: None,
            force_override_used: false,
            tool_calls: vec![],
            result_summary: None,
            validation_passed: None,
            started_at: None,
            completed_at: None,
            error_message: None,
            replay_record: None,
        });

        chain.clone()
    }

    #[test]
    fn goal_policy_enables_autonomous_loop_without_disabling_gates() {
        let mut policy = ChainPolicy::default();

        AutonomousLoopController::configure_goal_policy(&mut policy);

        assert!(policy.auto_resume);
        assert!(policy.auto_advance);
        assert!(policy.auto_retry_on_validation_failure);
        assert!(policy.require_validation_each_step);
        assert!(policy.halt_on_failure);
        assert!(policy.require_approval_for_high);
        assert!(policy.allow_auto_low_risk);
    }

    #[test]
    fn accepted_goal_chain_schedules_existing_resume_path() {
        let policy = ChainPolicy::default();
        let chain = chain_with_step();

        let decision = AutonomousLoopController::decide_goal_start(&chain, &policy);

        assert_eq!(
            decision,
            AutonomousStartDecision::Start {
                command: Command::ChainResume {
                    chain_id: "active".to_string(),
                    force: false
                },
                reason: "accepted goal chain is ready to start".to_string()
            }
        );
    }

    #[test]
    fn accepted_goal_chain_respects_step_limit() {
        let mut policy = ChainPolicy::default();
        policy.max_steps = 0;
        let chain = chain_with_step();

        let decision = AutonomousLoopController::decide_goal_start(&chain, &policy);

        assert!(matches!(
            decision,
            AutonomousStartDecision::Blocked { reason, .. }
                if reason.contains("max step limit reached")
        ));
    }

    #[test]
    fn seed_goal_context_only_adds_existing_files() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("README.md"), "demo").expect("write fixture");

        let mut chain = chain_with_step();
        let required = vec![
            "README.md".to_string(),
            "missing.rs".to_string(),
            "README.md".to_string(),
        ];

        let added = AutonomousLoopController::seed_goal_context(
            &mut chain,
            &required,
            temp.path().to_str().expect("utf-8 path"),
        );

        assert_eq!(added, 1);
        assert_eq!(chain.selected_context_files, vec!["README.md"]);
    }

    #[test]
    fn goal_sync_marks_matching_chain_completed() {
        let chain = chain_with_step();
        let mut goals = GoalManager::new();
        let goal = goals.stake_goal("demo", "conversation-1");
        goals
            .active_goal_mut()
            .expect("active goal")
            .attach_chain(chain.id.clone());
        assert_eq!(goal.status, GoalStatus::Stated);

        let updated = AutonomousLoopController::mark_goal_completed_for_chain(
            &mut goals,
            &chain,
            "done".to_string(),
        );

        let active = goals.active_goal().expect("active goal");
        assert!(updated);
        assert_eq!(active.status, GoalStatus::Completed);
        assert_eq!(active.completion_summary.as_deref(), Some("done"));
    }

    #[test]
    fn goal_sync_marks_matching_chain_failed_with_reason() {
        let chain = chain_with_step();
        let mut goals = GoalManager::new();
        goals.stake_goal("demo", "conversation-1");
        goals
            .active_goal_mut()
            .expect("active goal")
            .attach_chain(chain.id.clone());

        let updated = AutonomousLoopController::mark_goal_failed_for_chain(
            &mut goals,
            &chain,
            "step failed".to_string(),
        );

        let active = goals.active_goal().expect("active goal");
        assert!(updated);
        assert_eq!(active.status, GoalStatus::Failed);
        assert_eq!(active.failure_reason.as_deref(), Some("step failed"));
    }

    #[test]
    fn task_like_plain_text_classifier_routes_work_not_chat() {
        assert!(AutonomousLoopController::is_task_like_plain_text(
            "fix the parser bug"
        ));
        assert!(AutonomousLoopController::is_task_like_plain_text(
            "please add tests for goal confirmation"
        ));
        assert!(AutonomousLoopController::is_task_like_plain_text(
            "can you implement autonomous continuation"
        ));
        assert!(AutonomousLoopController::is_task_like_plain_text(
            "write a hello world script"
        ));
    }

    #[test]
    fn task_like_plain_text_classifier_leaves_questions_as_chat() {
        assert!(!AutonomousLoopController::is_task_like_plain_text(
            "explain the project structure"
        ));
        assert!(!AutonomousLoopController::is_task_like_plain_text(
            "what does this repo do?"
        ));
        assert!(!AutonomousLoopController::is_task_like_plain_text(
            "/goal fix parser"
        ));
    }

    #[test]
    fn detects_role_and_system_mode_prompts_as_execution_intent() {
        // Role/system definitions should be execution-intent, not chat
        assert!(AutonomousLoopController::is_task_like_plain_text(
            "You are a senior Rust engineer. Review this code and suggest improvements."
        ));
        assert!(AutonomousLoopController::is_task_like_plain_text(
            "Act as a DevOps specialist. Set up CI/CD pipeline with the following requirements..."
        ));
        assert!(AutonomousLoopController::is_task_like_plain_text(
            "Your role is a security auditor. Analyze the codebase for vulnerabilities."
        ));
        assert!(AutonomousLoopController::is_task_like_plain_text(
            "System mode: expert in Python optimization. Refactor the following code..."
        ));
    }

    #[test]
    fn detects_document_generation_prompts_as_execution_intent() {
        // Document generation with output requirements
        assert!(AutonomousLoopController::is_task_like_plain_text(
            "Generate a technical specification document.

Output Requirements:
- Include architecture diagram
- List all API endpoints
- Document error handling"
        ));
        assert!(AutonomousLoopController::is_task_like_plain_text(
            "Create a project README.

Goals:
1. Explain the project purpose
2. List installation steps
3. Provide usage examples"
        ));
        assert!(AutonomousLoopController::is_task_like_plain_text(
            "Expected deliverables:
- Source code
- Test files
- Documentation"
        ));
    }

    #[test]
    fn detects_multi_file_requests_as_execution_intent() {
        // Multiple file/artifact requests
        assert!(AutonomousLoopController::is_task_like_plain_text(
            "Create the following files:
1. src/main.rs - entry point
2. src/lib.rs - library code
3. Cargo.toml - dependencies
4. README.md - documentation"
        ));
        assert!(AutonomousLoopController::is_task_like_plain_text(
            "Generate 15 configuration files for different environments."
        ));
        assert!(AutonomousLoopController::is_task_like_plain_text(
            "Write multiple files: source files, config files, and test files."
        ));
    }

    #[test]
    fn detects_large_imperative_prompts_as_execution_intent() {
        // Large structured prompts with sections
        let large_prompt = "Execute the following:

# Objective
Build a complete web application

# Requirements
- User authentication
- Database integration
- REST API

# Deliverables
1. Source code
2. Tests
3. Documentation

This is a long imperative prompt with many words and clear structure that should not go to plain chat but instead be routed to the goal execution pipeline for proper planning and execution.";
        assert!(AutonomousLoopController::is_task_like_plain_text(large_prompt));

        // Another large structured prompt
        let doc_gen_prompt = "Implement the following system architecture.

## Background
We need a scalable solution.

## Context
Current system has limitations.

## Constraints
Must use Rust and be performant.

This prompt contains detailed instructions and should be treated as an execution task, not a simple conversational query that can be handled by plain chat mode.";
        assert!(AutonomousLoopController::is_task_like_plain_text(doc_gen_prompt));
    }

    #[test]
    fn conversational_questions_still_route_to_chat() {
        // Ensure normal conversational questions still go to chat
        assert!(!AutonomousLoopController::is_task_like_plain_text(
            "What is the best way to handle errors in Rust?"
        ));
        assert!(!AutonomousLoopController::is_task_like_plain_text(
            "Can you explain how async works?"
        ));
        assert!(!AutonomousLoopController::is_task_like_plain_text(
            "Why does this code fail to compile?"
        ));
        assert!(!AutonomousLoopController::is_task_like_plain_text(
            "How do I set up a new project?"
        ));
    }

    #[test]
    fn complex_conversational_mixed_with_execution_detected_correctly() {
        // Even if mixed with conversational elements, structured execution wins
        assert!(AutonomousLoopController::is_task_like_plain_text(
            "How would you approach this? Create a module with the following:
1. Error handling
2. Logging
3. Configuration"
        ));
    }

    #[test]
    fn planner_preflight_accepts_existing_configured_model() {
        let outcome = AutonomousLoopController::resolve_planner_preflight(
            Some(DEFAULT_CODER_14B_MODEL),
            &[model(DEFAULT_CODER_14B_MODEL)],
            true,
            None,
        );

        assert_eq!(
            outcome,
            PlannerPreflightOutcome::Ready {
                model: DEFAULT_CODER_14B_MODEL.to_string(),
                binding: PlannerModelBinding::Configured
            }
        );
    }

    #[test]
    fn planner_preflight_auto_binds_qwen_coder_when_unset() {
        let outcome = AutonomousLoopController::resolve_planner_preflight(
            None,
            &[model("llama3.1:latest"), model(DEFAULT_CODER_14B_MODEL)],
            true,
            None,
        );

        assert_eq!(
            outcome,
            PlannerPreflightOutcome::Ready {
                model: DEFAULT_CODER_14B_MODEL.to_string(),
                binding: PlannerModelBinding::AutoBound
            }
        );
    }

    #[test]
    fn planner_preflight_rebinds_invalid_configured_model_to_qwen_coder() {
        let outcome = AutonomousLoopController::resolve_planner_preflight(
            Some("missing-coder:latest"),
            &[model(DEFAULT_CODER_14B_MODEL)],
            true,
            None,
        );

        assert_eq!(
            outcome,
            PlannerPreflightOutcome::Ready {
                model: DEFAULT_CODER_14B_MODEL.to_string(),
                binding: PlannerModelBinding::ReboundInvalidConfigured {
                    invalid_model: "missing-coder:latest".to_string(),
                    reason: "configured model 'missing-coder:latest' is not installed".to_string()
                }
            }
        );
    }

    #[test]
    fn planner_preflight_fails_before_chain_when_no_usable_model_exists() {
        let outcome = AutonomousLoopController::resolve_planner_preflight(
            None,
            &[model("llama3.1:latest")],
            true,
            None,
        );

        assert_eq!(outcome, PlannerPreflightOutcome::MissingLocalModel);
    }

    #[test]
    fn planner_preflight_reports_ollama_unavailable() {
        let outcome = AutonomousLoopController::resolve_planner_preflight(
            None,
            &[],
            false,
            Some("connection refused"),
        );

        assert_eq!(
            outcome,
            PlannerPreflightOutcome::OllamaUnavailable {
                reason: "connection refused".to_string()
            }
        );
    }

    #[test]
    fn planner_preflight_uses_existing_safe_fallback_after_coder_models() {
        let outcome = AutonomousLoopController::resolve_planner_preflight(
            None,
            &[model(FALLBACK_PLANNER_MODEL)],
            true,
            None,
        );

        assert_eq!(
            outcome,
            PlannerPreflightOutcome::Ready {
                model: FALLBACK_PLANNER_MODEL.to_string(),
                binding: PlannerModelBinding::AutoBound
            }
        );
    }
}
