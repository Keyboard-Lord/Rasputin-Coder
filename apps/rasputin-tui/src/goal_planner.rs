use crate::guidance::{
    GeneratedPlan, Goal, OutcomePrediction, PlanStep, Risk, RiskLevel, RiskType, StepActionType,
};
use crate::ollama::ChatMessage;
use serde::Deserialize;
use std::collections::HashSet;

const MAX_MODEL_STEPS: usize = 12;

pub struct QwenGoalPlanner;

#[derive(Debug, Deserialize)]
struct ModelPlan {
    objective: Option<String>,
    steps: Vec<ModelStep>,
    #[serde(default)]
    risks: Vec<ModelRisk>,
    #[serde(default)]
    required_context: Vec<String>,
    reasoning: Option<String>,
    safe_to_chain: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct ModelStep {
    description: String,
    #[serde(default)]
    action_type: String,
    #[serde(default)]
    risk_level: String,
    #[serde(default)]
    likely_approval_needed: bool,
    #[serde(default)]
    affected_files: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ModelRisk {
    #[serde(default)]
    risk_type: String,
    description: String,
    #[serde(default)]
    affected: Vec<String>,
    #[serde(default)]
    mitigation: String,
    #[serde(default)]
    level: String,
}

impl QwenGoalPlanner {
    pub fn build_messages(
        goal: &Goal,
        repo_evidence: &str,
        previous_plan: Option<&GeneratedPlan>,
        working_context: Option<&str>,
    ) -> Vec<ChatMessage> {
        let previous_plan_summary = previous_plan
            .map(Self::format_previous_plan)
            .unwrap_or_else(|| "None".to_string());

        // Build user prompt with optional working memory context
        let user_content = if let Some(context) = working_context {
            format!(
                "Raw goal prompt (verbatim):\n{}\n\n{}\n\nRepository evidence:\n{}\n\nPrevious rejected plan:\n{}\n\nReturn the plan JSON now.",
                goal.statement, context, repo_evidence, previous_plan_summary
            )
        } else {
            format!(
                "Raw goal prompt (verbatim):\n{}\n\nRepository evidence:\n{}\n\nPrevious rejected plan:\n{}\n\nReturn the plan JSON now.",
                goal.statement, repo_evidence, previous_plan_summary
            )
        };

        vec![
            ChatMessage {
                role: "system".to_string(),
                content: Self::system_prompt().to_string(),
            },
            ChatMessage {
                role: "user".to_string(),
                content: user_content,
            },
        ]
    }

    pub fn parse_response(
        response: &str,
        raw_prompt: &str,
    ) -> Result<GeneratedPlan, String> {
        let json = extract_json_object(response)?;
        let model_plan: ModelPlan = serde_json::from_str(json)
            .map_err(|err| format!("model plan JSON did not match schema: {}", err))?;

        if model_plan.steps.is_empty() {
            return Err("model plan contained no steps".to_string());
        }

        let mut steps = Vec::new();
        for (idx, step) in model_plan
            .steps
            .into_iter()
            .take(MAX_MODEL_STEPS)
            .enumerate()
        {
            let description = step.description.trim();
            if description.is_empty() {
                return Err(format!("step {} has an empty description", idx + 1));
            }

            let risk_level = parse_risk_level(&step.risk_level);
            steps.push(PlanStep {
                number: idx + 1,
                description: description.to_string(),
                action_type: parse_action_type(&step.action_type, description),
                risk_level,
                likely_approval_needed: step.likely_approval_needed
                    || matches!(risk_level, RiskLevel::Critical),
                affected_files: normalize_file_list(step.affected_files),
            });
        }

        let risks: Vec<Risk> = model_plan
            .risks
            .into_iter()
            .map(|risk| Risk {
                risk_type: parse_risk_type(&risk.risk_type),
                description: non_empty_or(risk.description, "Model reported an execution risk"),
                affected: normalize_file_list(risk.affected),
                mitigation: non_empty_or(risk.mitigation, "Review before continuing"),
                level: parse_risk_level(&risk.level),
            })
            .collect();

        let mut required_context = normalize_file_list(model_plan.required_context);
        for step in &steps {
            for file in &step.affected_files {
                if !required_context.contains(file) {
                    required_context.push(file.clone());
                }
            }
        }

        let approval_points: Vec<usize> = steps
            .iter()
            .filter(|step| {
                step.likely_approval_needed || matches!(step.risk_level, RiskLevel::Critical)
            })
            .map(|step| step.number)
            .collect();

        let has_critical = steps
            .iter()
            .any(|step| matches!(step.risk_level, RiskLevel::Critical))
            || risks
                .iter()
                .any(|risk| matches!(risk.level, RiskLevel::Critical));
        let has_warning = steps
            .iter()
            .any(|step| matches!(step.risk_level, RiskLevel::Warning))
            || risks
                .iter()
                .any(|risk| matches!(risk.level, RiskLevel::Warning));

        let safe_to_chain =
            model_plan.safe_to_chain.unwrap_or(true) && !has_critical && approval_points.is_empty();

        let fallback_objective = crate::guidance::summarize_goal_objective(raw_prompt);
        let parsed_objective = model_plan
            .objective
            .filter(|objective| !objective.trim().is_empty())
            .unwrap_or_else(|| fallback_objective.clone());
        let objective = if crate::guidance::is_vague_objective_summary(&parsed_objective) {
            fallback_objective
        } else {
            parsed_objective
        };

        Ok(GeneratedPlan {
            raw_prompt: raw_prompt.to_string(),
            objective,
            steps,
            risks,
            approval_points,
            required_context,
            estimated_outcome: if has_critical {
                OutcomePrediction::Uncertain
            } else if has_warning {
                OutcomePrediction::SuccessWithWarnings
            } else {
                OutcomePrediction::Success
            },
            safe_to_chain,
            reasoning: model_plan
                .reasoning
                .filter(|reasoning| !reasoning.trim().is_empty())
                .unwrap_or_else(|| {
                    "Qwen-Coder generated this plan from the goal and repository evidence."
                        .to_string()
                }),
        })
    }

    fn system_prompt() -> &'static str {
        "You are Qwen-Coder acting as Rasputin's autonomous SWE planner. \
Return only one JSON object. Do not use markdown. \
Schema: {\"objective\":\"short objective\",\"steps\":[{\"description\":\"imperative step\",\"action_type\":\"read|write|execute|validate|commit|external\",\"risk_level\":\"safe|caution|warning|critical\",\"likely_approval_needed\":false,\"affected_files\":[\"path\"]}],\"risks\":[{\"risk_type\":\"git_conflict|validation_failure|missing_context|approval_required|unprotected_write|external_dependency|mode_limitation\",\"description\":\"risk\",\"affected\":[\"path\"],\"mitigation\":\"mitigation\",\"level\":\"safe|caution|warning|critical\"}],\"required_context\":[\"path\"],\"reasoning\":\"why this sequence is correct\",\"safe_to_chain\":true}. \
Keep the plan bounded to 3-8 concrete steps. Preserve validation and approval gates. \
Do not invent files; include file paths only when present in repository evidence or clearly implied by the goal. \
The objective must preserve explicit deliverable contracts from the raw prompt: exact counts, artifact type, and exact filenames when provided. \
Do not collapse the objective into vague phrases like 'Analyze requirements' or 'Begin your analysis now'. \
If WORKING CONTEXT is provided, it represents the current session state including completed work, remaining deliverables, and recent files. Use this to maintain continuity across turns."
    }

    fn format_previous_plan(plan: &GeneratedPlan) -> String {
        let mut summary = format!(
            "{} step(s), {} risk(s), {} approval point(s)",
            plan.steps.len(),
            plan.risks.len(),
            plan.approval_points.len()
        );
        for step in plan.steps.iter().take(5) {
            summary.push_str(&format!("\n{}. {}", step.number, step.description));
        }
        summary
    }
}

fn extract_json_object(response: &str) -> Result<&str, String> {
    let start = response
        .find('{')
        .ok_or_else(|| "model response did not contain a JSON object".to_string())?;
    let end = response
        .rfind('}')
        .ok_or_else(|| "model response did not close a JSON object".to_string())?;

    if end < start {
        return Err("model response JSON object was malformed".to_string());
    }

    Ok(&response[start..=end])
}

fn parse_action_type(value: &str, description: &str) -> StepActionType {
    match value.trim().to_lowercase().as_str() {
        "read" | "inspect" | "analyze" | "review" => return StepActionType::Read,
        "write" | "edit" | "modify" | "implement" | "fix" | "create" | "refactor" => {
            return StepActionType::Write;
        }
        "execute" | "run" | "shell" | "command" => return StepActionType::Execute,
        "validate" | "test" | "check" | "verify" => return StepActionType::Validate,
        "commit" => return StepActionType::Commit,
        "external" => return StepActionType::External,
        _ => {}
    }

    let combined = description.to_lowercase();
    if combined.contains("commit") {
        StepActionType::Commit
    } else if combined.contains("validate")
        || combined.contains("test")
        || combined.contains("check")
        || combined.contains("verify")
    {
        StepActionType::Validate
    } else if combined.contains("execute")
        || combined.contains("run")
        || combined.contains("command")
        || combined.contains("shell")
    {
        StepActionType::Execute
    } else if combined.contains("external") || combined.contains("network") {
        StepActionType::External
    } else if combined.contains("write")
        || combined.contains("edit")
        || combined.contains("modify")
        || combined.contains("implement")
        || combined.contains("fix")
        || combined.contains("add")
        || combined.contains("create")
        || combined.contains("refactor")
    {
        StepActionType::Write
    } else {
        StepActionType::Read
    }
}

fn parse_risk_level(value: &str) -> RiskLevel {
    match value.trim().to_lowercase().as_str() {
        "critical" | "high" | "blocker" => RiskLevel::Critical,
        "warning" | "warn" => RiskLevel::Warning,
        "caution" | "medium" | "moderate" => RiskLevel::Caution,
        _ => RiskLevel::Safe,
    }
}

fn parse_risk_type(value: &str) -> RiskType {
    match value.trim().to_lowercase().as_str() {
        "git_conflict" | "git conflict" => RiskType::GitConflict,
        "missing_context" | "missing context" => RiskType::MissingContext,
        "approval_required" | "approval required" => RiskType::ApprovalRequired,
        "unprotected_write" | "unprotected write" => RiskType::UnprotectedWrite,
        "external_dependency" | "external dependency" => RiskType::ExternalDependency,
        "mode_limitation" | "mode limitation" => RiskType::ModeLimitation,
        _ => RiskType::ValidationFailure,
    }
}

fn normalize_file_list(files: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut normalized = Vec::new();

    for file in files {
        let file = file.trim();
        if file.is_empty() || file.starts_with('/') {
            continue;
        }
        if seen.insert(file.to_string()) {
            normalized.push(file.to_string());
        }
    }

    normalized
}

fn non_empty_or(value: String, fallback: &str) -> String {
    if value.trim().is_empty() {
        fallback.to_string()
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_qwen_plan_json_into_generated_plan() {
        let response = r#"{
            "objective": "Harden parser",
            "steps": [
                {
                    "description": "Inspect command parser tests",
                    "action_type": "read",
                    "risk_level": "safe",
                    "likely_approval_needed": false,
                    "affected_files": ["apps/rasputin-tui/src/commands.rs"]
                },
                {
                    "description": "Modify parser routing",
                    "action_type": "write",
                    "risk_level": "caution",
                    "likely_approval_needed": false,
                    "affected_files": ["apps/rasputin-tui/src/commands.rs"]
                }
            ],
            "risks": [],
            "required_context": ["apps/rasputin-tui/src/commands.rs"],
            "reasoning": "Parser ambiguity must be removed before execution.",
            "safe_to_chain": true
        }"#;

        let raw_prompt = "Fix parser ambiguity in apps/rasputin-tui/src/commands.rs";
        let plan = QwenGoalPlanner::parse_response(response, raw_prompt).expect("plan");

        assert_eq!(plan.objective, "Harden parser");
        assert_eq!(plan.raw_prompt, raw_prompt);
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.steps[0].action_type, StepActionType::Read);
        assert_eq!(plan.steps[1].risk_level, RiskLevel::Caution);
        assert!(plan.safe_to_chain);
        assert_eq!(
            plan.required_context,
            vec!["apps/rasputin-tui/src/commands.rs"]
        );
    }

    #[test]
    fn rejects_empty_model_plan() {
        let response = r#"{"objective":"empty","steps":[]}"#;

        let err = QwenGoalPlanner::parse_response(response, "fallback").expect_err("error");

        assert!(err.contains("no steps"));
    }

    #[test]
    fn extracts_json_from_fenced_response() {
        let response = "```json\n{\"steps\":[{\"description\":\"Validate build\",\"action_type\":\"validate\"}]}\n```";
        let raw_prompt = "Fix fallback parser";
        let plan = QwenGoalPlanner::parse_response(response, raw_prompt).expect("plan");

        assert_eq!(plan.steps[0].action_type, StepActionType::Validate);
        assert_eq!(plan.objective, raw_prompt);
    }

    #[test]
    fn replaces_vague_model_objective_with_contract_summary() {
        let response = r#"{
            "objective": "Analyze goal requirements",
            "steps": [
                {
                    "description": "Create docs/01_PROJECT_OVERVIEW.md",
                    "action_type": "write",
                    "risk_level": "safe",
                    "likely_approval_needed": false,
                    "affected_files": ["docs/01_PROJECT_OVERVIEW.md"]
                }
            ],
            "risks": [],
            "required_context": ["docs/01_PROJECT_OVERVIEW.md"],
            "reasoning": "Write the requested file first.",
            "safe_to_chain": true
        }"#;
        let raw_prompt = "Create exactly 2 markdown files with these precise filenames:\n1. docs/01_PROJECT_OVERVIEW.md\n2. docs/02_ARCHITECTURE.md";

        let plan = QwenGoalPlanner::parse_response(response, raw_prompt).expect("plan");

        assert_eq!(
            plan.objective,
            "Generate exactly 2 markdown file(s) with the specified filenames"
        );
    }
}
