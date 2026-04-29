//! Deep Forge: Air-gapped, self-exhausting code refinement engine
//! 
//! Usage: rasputin-forge --target <path_to_repo>

use clap::Parser;
use std::path::PathBuf;
use tracing::{error, info};

mod types;
mod critic;
mod chisel;
mod ollama;
mod forge_loop;
mod git;

use types::ForgeConfig;
use forge_loop::ExhaustionLoop;

/// Deep Forge CLI arguments
#[derive(Parser, Debug)]
#[command(name = "rasputin-forge")]
#[command(about = "Air-gapped, self-exhausting code refinement engine")]
#[command(version)]
struct Args {
    /// Target repository path
    #[arg(short, long, value_name = "PATH")]
    target: PathBuf,
    
    /// Ollama model to use
    #[arg(short, long, default_value = "qwen2.5-coder:14b")]
    model: String,
    
    /// Ollama API endpoint
    #[arg(long, default_value = "http://localhost:11434/api/generate")]
    ollama_endpoint: String,
    
    /// Timeout for Ollama requests (seconds)
    #[arg(long, default_value_t = 300)]
    timeout: u64,
    
    /// Maximum iterations before declaring stalled
    #[arg(long, default_value_t = 100)]
    max_iterations: usize,
    
    /// Disable auto-commit
    #[arg(long)]
    no_commit: bool,
    
    /// Test command to run
    #[arg(long, default_value = "cargo test")]
    test_command: String,
    
    /// Linter commands (comma-separated)
    #[arg(long, default_value = "cargo clippy --all-targets --all-features -- -D warnings")]
    linters: String,
}

#[tokio::main]
async fn main() {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter("rasputin_forge=info,critic=info,chisel=info,ollama=info,forge_loop=info")
        .init();
    
    // Parse arguments
    let args = Args::parse();
    
    // Build configuration
    let config = ForgeConfig {
        target_repo: args.target.clone(),
        ollama_endpoint: args.ollama_endpoint.clone(),
        model: args.model.clone(),
        ollama_timeout: args.timeout,
        max_iterations: args.max_iterations,
        auto_commit: !args.no_commit,
        linters: args.linters.split(',').map(|s| s.trim().to_string()).collect(),
        test_command: args.test_command.clone(),
    };
    
    // Validate target exists
    if !args.target.exists() {
        eprintln!("[FATAL] Target path does not exist: {:?}", args.target);
        std::process::exit(1);
    }
    
    // Validate it's a git repository
    let git_path = args.target.join(".git");
    if !git_path.exists() {
        eprintln!("[FATAL] Target is not a git repository: {:?}", args.target);
        eprintln!("        Initialize with: git init");
        std::process::exit(1);
    }
    
    info!("[DEEP FORGE] Target: {:?}", args.target);
    info!("[DEEP FORGE] Model: {}", args.model);
    info!("[DEEP FORGE] Ollama: {}", args.ollama_endpoint);
    
    // Print banner
    print_banner();
    
    // Run the Masterpiece Loop
    match run_forge(config).await {
        Ok(stats) => {
            println!("\n[FORGE COMPLETE]");
            println!("Iterations: {}", stats.iterations);
            println!("Flaws detected: {}", stats.flaws_detected);
            println!("Flaws fixed: {}", stats.flaws_fixed);
            println!("Patches applied: {}", stats.patches_applied);
            println!("Tests passed: {}", stats.tests_passed);
            println!("Tests failed: {}", stats.tests_failed);
            
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("\n[FATAL ERROR] {}", e);
            std::process::exit(1);
        }
    }
}

/// Run the Deep Forge
async fn run_forge(config: ForgeConfig) -> Result<types::ForgeStats, types::ForgeError> {
    // Create backup branch
    let git = git::GitOps::new(config.target_repo.clone())?;
    git.create_backup_branch().await?;
    
    // Initialize and run exhaustion loop
    let mut forge = ExhaustionLoop::new(config).await?;
    forge.run().await
}

/// Print the Deep Forge banner
fn print_banner() {
    println!(r#"
    ╔══════════════════════════════════════════════════════════╗
    ║                                                          ║
    ║           D E E P   F O R G E   A C T I V A T E D        ║
    ║                                                          ║
    ║     Air-Gapped Code Refinement Engine                   ║
    ║     Zero Cloud Dependency | 100% Local Operation          ║
    ║                                                          ║
    ║     Mode: SELF-EXHAUSTING                               ║
    ║     Loop: Until No Flaws Remain                         ║
    ║                                                          ║
    ╚══════════════════════════════════════════════════════════╝
    "#);
}
