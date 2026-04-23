//! Forge internal worker entrypoint.
//!
//! This binary is launched by `rasputin-tui` to execute one bounded Forge task.

#![warn(warnings)]
#![allow(dead_code)]

#[path = "../../../support/workspace_config.rs"]
mod workspace_config;

mod approval_checkpoint;
mod chain_executor;
mod chain_registry;
mod context_assembly;
mod crypto_hash;
mod determinism_guard;
mod execution;
mod git_grounding;
mod governance;
mod observability;
mod planner;
mod planner_attack_fixtures;
mod planner_envelope;
mod runtime;
mod runtime_gates;
mod state;
mod system_invariants;
mod task_intake;
mod tool_registry;
mod tools;
mod types;
mod validator;

#[cfg(test)]
mod chain_fixtures;
#[cfg(test)]
mod chain_replay_tests;
#[cfg(test)]
mod conformance_tests;
#[cfg(test)]
mod replay_seal_tests;
#[cfg(test)]
mod state_hash_tests;
#[cfg(test)]
mod tool_governance_tests;
#[cfg(test)]
mod validation_matrix_tests;
use planner::model_http::{DEFAULT_CODER_14B_MODEL, normalize_requested_model};
use runtime::{RuntimeConfig, run_bootstrap};
use std::env;

const DEFAULT_TASK: &str = "Create a hello.txt file with 'hello world' content";
const DEFAULT_MAX_ITERATIONS: u32 = 10;
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    // Parse command line arguments
    let args: Vec<String> = env::args().collect();

    // Check for version
    if args.len() > 1 && (args[1] == "--version" || args[1] == "-v") {
        println!("Forge v{}", VERSION);
        return;
    }

    // Check for help
    if args.len() > 1 && (args[1] == "--help" || args[1] == "-h") {
        print_help();
        return;
    }

    if !structured_output_only() {
        println!("{}", "=".repeat(60));
        println!("FORGE v{}: Terminal Agent Runtime", VERSION);
        println!("{}", "=".repeat(60));
        println!();
    }

    let task = args.get(1).map(|s| s.as_str()).unwrap_or(DEFAULT_TASK);
    let max_iterations = args
        .get(2)
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(DEFAULT_MAX_ITERATIONS);
    let planner_type = args.get(3).map(|s| s.as_str()).unwrap_or("http");
    let planner_model = env_override("FORGE_PLANNER_MODEL")
        .map(|model| {
            let normalized = normalize_requested_model(&model);
            if !structured_output_only() {
                println!(
                    "Planner model source: FORGE_PLANNER_MODEL -> {}",
                    normalized
                );
            }
            normalized
        })
        .or_else(|| {
            workspace_planner_model().map(|(model, source)| {
                if !structured_output_only() {
                    println!("Planner model source: {} -> {}", source, model);
                }
                model
            })
        })
        .unwrap_or_else(|| DEFAULT_CODER_14B_MODEL.to_string());
    let planner_endpoint = env_override("FORGE_PLANNER_ENDPOINT")
        .unwrap_or_else(|| "http://127.0.0.1:11434".to_string());
    let planner_temperature = env_override("FORGE_PLANNER_TEMPERATURE")
        .and_then(|value| value.parse::<f32>().ok())
        .unwrap_or(0.0);
    let planner_seed = env_override("FORGE_PLANNER_SEED")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(42);
    let css_compression = env_override("FORGE_CSS_COMPRESSION")
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false);

    let config = RuntimeConfig {
        max_iterations,
        task: task.to_string(),
        auto_revert: true,
        mode: types::ExecutionMode::Edit,
        planner_type: planner_type.to_string(),
        planner_endpoint,
        planner_model,
        planner_timeout_seconds: 30,
        planner_temperature,
        planner_seed,
        css_compression,
    };

    if !structured_output_only() {
        println!("Task: {}", config.task);
        println!("Max iterations: {}", config.max_iterations);
        println!("Mode: {:?}", config.mode);
        println!("Planner: {}", config.planner_type);
        println!();
        println!("{}", "-".repeat(60));
    }

    // Run the bootstrap
    let result = run_bootstrap(config);

    if !structured_output_only() {
        println!();
        println!("{}", "-".repeat(60));
        println!();
        println!("EXECUTION SUMMARY");
        println!("{}", "=".repeat(60));
        println!("Success: {}", result.success);
        println!("Iterations: {}", result.iterations);
        println!("Final status: {:?}", result.final_state.status);

        let files_str = if result.final_state.files_written.is_empty() {
            "none".to_string()
        } else {
            result
                .final_state
                .files_written
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };
        println!("Files written: {}", files_str);

        if let Some(ref reason) = result.final_state.completion_reason {
            println!("Completion reason: {}", reason.as_str());
        }

        if let Some(ref error) = result.error {
            println!("Error: {}", error);
        }

        println!();
    }

    // Exit with appropriate code
    std::process::exit(if result.success { 0 } else { 1 });
}

fn workspace_planner_model() -> Option<(String, &'static str)> {
    workspace_config::discover_workspace_model(&env::current_dir().ok()?)
        .map(|config| (normalize_requested_model(&config.model), config.source))
}

fn env_override(key: &str) -> Option<String> {
    env::var(key).ok().filter(|value| !value.trim().is_empty())
}

fn structured_output_only() -> bool {
    matches!(
        env_override("FORGE_OUTPUT_MODE").as_deref(),
        Some("jsonl") | Some("JSONL")
    )
}

fn print_help() {
    println!("Forge v{} - Internal worker runtime", VERSION);
    println!();
    println!("USAGE:");
    println!("    forge_bootstrap [OPTIONS] [TASK] [MAX_ITERATIONS] [PLANNER_TYPE]");
    println!();
    println!("OPTIONS:");
    println!("    -h, --help          Print this help message");
    println!("    -v, --version       Print version information");
    println!();
    println!("ENVIRONMENT:");
    println!("    FORGE_PLANNER_MODEL         Model to use (default: qwen2.5-coder:14b)");
    println!("    FORGE_PLANNER_ENDPOINT    Ollama endpoint (default: http://127.0.0.1:11434)");
    println!("    FORGE_PLANNER_TEMPERATURE   Temperature 0.0-0.1 (default: 0.0)");
    println!("    FORGE_PLANNER_SEED          Random seed (default: 42)");
    println!("    FORGE_CSS_COMPRESSION       Enable CSS compression (default: auto)");
    println!("    FORGE_OUTPUT_MODE           Output format: jsonl (default: human)");
    println!();
    println!("EXAMPLES:");
    println!("    forge_bootstrap 'Create hello.txt' 10 http");
    println!();
    println!("NOTES:");
    println!("    This binary is an internal worker launched by rasputin-tui.");
}
