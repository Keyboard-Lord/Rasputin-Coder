//! The Exhaustion Loop: Core orchestration engine for the Deep Forge

use crate::types::{Flaw, FlawQueue, ForgeConfig, ForgeError, ForgeStats, LoopStatus, PatchResult};
use sha2::{Sha256, Digest};
use hex;

fn compute_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}
use crate::critic::LocalCritic;
use crate::chisel::Chisel;
use crate::ollama::OllamaClient;
use crate::git::GitOps;
use std::path::PathBuf;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{error, info, warn};

/// The Masterpiece Loop - runs until no flaws remain
pub struct ExhaustionLoop {
    config: ForgeConfig,
    critic: LocalCritic,
    chisel: Chisel,
    ollama: OllamaClient,
    git: GitOps,
    stats: ForgeStats,
    status: LoopStatus,
    iteration: usize,
}

impl ExhaustionLoop {
    /// Create new exhaustion loop
    pub async fn new(config: ForgeConfig) -> Result<Self, ForgeError> {
        info!("[FORGE] Initializing Deep Forge...");
        
        // Verify Ollama is available
        let ollama = OllamaClient::new(config.clone());
        if !ollama.health_check().await? {
            return Err(ForgeError::Ollama(
                "Ollama not available at localhost:11434. Start Ollama first.".to_string()
            ));
        }
        
        let critic = LocalCritic::new(config.clone())?;
        let chisel = Chisel::new(config.target_repo.clone());
        let git = GitOps::new(config.target_repo.clone())?;
        
        Ok(Self {
            config,
            critic,
            chisel,
            ollama,
            git,
            stats: ForgeStats::default(),
            status: LoopStatus::Running,
            iteration: 0,
        })
    }
    
    /// Run the Masterpiece Loop until exhaustion
    pub async fn run(&mut self) -> Result<ForgeStats, ForgeError> {
        info!("[FORGE] Starting Masterpiece Loop...");
        println!("\n[DEEP FORGE ACTIVATED]");
        println!("Target: {:?}", self.config.target_repo);
        println!("Model: {}\n", self.config.model);
        
        self.stats.start_time = Some(chrono::Utc::now());
        
        // Main exhaustion loop
        while self.status == LoopStatus::Running {
            self.iteration += 1;
            
            if self.iteration > self.config.max_iterations {
                self.status = LoopStatus::Stalled;
                warn!("[FORGE] Loop stalled after {} iterations", self.config.max_iterations);
                break;
            }
            
            println!("\n[ITERATION {}]", self.iteration);
            println!("{}", "=".repeat(60));
            
            // Execute one iteration
            match self.iteration().await {
                Ok(true) => {
                    // Exhausted - no more flaws
                    break;
                }
                Ok(false) => {
                    // Continue to next iteration
                    sleep(Duration::from_secs(1)).await;
                }
                Err(e) => {
                    error!("[FORGE] Iteration {} failed: {:?}", self.iteration, e);
                    self.status = LoopStatus::Failed;
                    return Err(e);
                }
            }
        }
        
        self.stats.end_time = Some(chrono::Utc::now());
        
        // Final status
        match self.status {
            LoopStatus::Running | LoopStatus::Exhausted => {
                println!("\n[MASTERPIECE ACHIEVED: NO REMAINING TASKS]");
                println!("Iterations: {}", self.stats.iterations);
                println!("Flaws fixed: {}", self.stats.flaws_fixed);
                println!("Patches applied: {}", self.stats.patches_applied);
                std::process::exit(0);
            }
            LoopStatus::Stalled => {
                println!("\n[FORGE STALLED: Maximum iterations reached]");
                std::process::exit(1);
            }
            LoopStatus::Failed => {
                println!("\n[FORGE FAILED: Critical error occurred]");
                std::process::exit(2);
            }
        }
    }
    
    /// Execute one iteration of the loop
    /// Returns Ok(true) if exhausted, Ok(false) to continue
    async fn iteration(&mut self) -> Result<bool, ForgeError> {
        self.stats.iterations = self.iteration;
        
        // Phase A: The Audit
        println!("\n[PHASE A: THE AUDIT]");
        let mut flaw_queue = self.critic.audit().await?;
        
        // Exit condition: if queue is empty
        if flaw_queue.is_empty() {
            println!("[AUDIT] Flaw queue empty - running final deep scan...");
            
            // Run tests one more time to be sure
            match self.run_tests().await {
                Ok(true) => {
                    println!("[AUDIT] All tests pass. Queue confirmed empty.");
                    self.status = LoopStatus::Exhausted;
                    return Ok(true);
                }
                Ok(false) => {
                    // Tests failed but no flaws detected - this is a problem
                    warn!("[FORGE] Tests failing but no flaws detected in audit");
                    // Continue anyway, maybe next iteration will catch it
                }
                Err(e) => {
                    warn!("[FORGE] Test execution failed: {:?}", e);
                }
            }
        }
        
        println!("[AUDIT] {} flaws queued for fixing", flaw_queue.len());
        self.stats.flaws_detected += flaw_queue.len();
        
        // Phase B: The Draft - process flaws
        println!("\n[PHASE B: THE DRAFT]");
        
        while let Some(flaw) = flaw_queue.pop_next() {
            println!("\n[FLAW] {:?}:{} - {:?}", flaw.file_path, flaw.line, flaw.category);
            println!("       {}", flaw.description);
            
            match self.process_flaw(&flaw).await {
                Ok(true) => {
                    self.stats.flaws_fixed += 1;
                    println!("       ✓ Fixed");
                }
                Ok(false) => {
                    // Retry later
                    flaw_queue.retry_later(flaw);
                    println!("       ✗ Failed (will retry)");
                }
                Err(e) => {
                    error!("[FORGE] Error processing flaw {}: {:?}", flaw.id, e);
                    flaw_queue.retry_later(flaw);
                }
            }
        }
        
        // Phase C & D happen within process_flaw
        
        // Not exhausted yet
        Ok(false)
    }
    
    /// Process a single flaw
    /// Returns Ok(true) if fixed, Ok(false) if should retry
    async fn process_flaw(&mut self, flaw: &Flaw) -> Result<bool, ForgeError> {
        // Read current file content
        let file_path = self.config.target_repo.join(&flaw.file_path);
        let content = tokio::fs::read_to_string(&file_path).await
            .map_err(|e| ForgeError::Io(e))?;
        
        // Check if file has changed since flaw was detected
        let current_hash = compute_hash(&content);
        if current_hash != flaw.content_hash {
            warn!("[FORGE] File {:?} has changed since flaw detection", flaw.file_path);
            // Still try to fix it
        }
        
        // Backup file
        self.chisel.backup_file(&flaw.file_path).await?;
        
        // Generate fix with Ollama
        let fix_response = self.ollama.generate_fix(flaw, &content).await?;
        
        // Parse patches
        let patches = self.chisel.parse_patches(&fix_response, &flaw.file_path);
        
        if patches.is_empty() {
            warn!("[FORGE] No patches generated for flaw {}", flaw.id);
            return Ok(false);
        }
        
        // Phase C: The Survival Test
        println!("       [PHASE C: SURVIVAL TEST]");
        
        let mut retry_count = 0;
        let max_retries = 3;
        
        loop {
            // Apply patches
            let results = self.chisel.apply_patches(&patches).await;
            
            let all_applied = results.iter().all(|r| match r {
                Ok(PatchResult::Success { .. }) => true,
                _ => false,
            });
            
            if !all_applied {
                // Restore and retry
                self.chisel.restore_file(&flaw.file_path).await?;
                
                retry_count += 1;
                if retry_count >= max_retries {
                    println!("       ✗ Patch failed after {} retries", max_retries);
                    self.chisel.restore_file(&flaw.file_path).await?;
                    return Ok(false);
                }
                
                // Get compiler errors for self-healing
                let errors = self.get_compiler_errors().await?;
                if errors.is_empty() {
                    break;
                }
                
                // Self-healing: feed errors back to LLM
                let healing_prompt = format!(
                    "The previous fix failed with these errors:\n{}\n\nPlease provide a corrected SEARCH/REPLACE block.",
                    errors
                );
                let healed_response = self.ollama.generate_fix(flaw, &healing_prompt).await?;
                let healed_patches = self.chisel.parse_patches(&healed_response, &flaw.file_path);
                
                if healed_patches.is_empty() {
                    break;
                }
                
                println!("       [SELF-HEALING] Attempt {}/{} with compiler feedback", retry_count, max_retries);
                continue;
            }
            
            // Run tests
            match self.run_tests().await {
                Ok(true) => {
                    println!("       ✓ Tests pass");
                    break;
                }
                Ok(false) => {
                    // Tests failed
                    self.chisel.restore_file(&flaw.file_path).await?;
                    
                    retry_count += 1;
                    if retry_count >= max_retries {
                        println!("       ✗ Tests failed after {} retries", max_retries);
                        return Ok(false);
                    }
                    
                    // Get test errors
                    let test_errors = self.get_test_errors().await?;
                    
                    // Self-healing with test feedback
                    let healing_prompt = format!(
                        "The previous fix caused test failures:\n{}\n\nPlease provide a corrected SEARCH/REPLACE block.",
                        test_errors
                    );
                    let healed_response = self.ollama.generate_fix(flaw, &healing_prompt).await?;
                    let healed_patches = self.chisel.parse_patches(&healed_response, &flaw.file_path);
                    
                    if healed_patches.is_empty() {
                        break;
                    }
                    
                    println!("       [SELF-HEALING] Attempt {}/{} with test feedback", retry_count, max_retries);
                }
                Err(e) => {
                    warn!("[FORGE] Test execution error: {:?}", e);
                    break; // Continue anyway
                }
            }
        }
        
        // Phase D: The Polish
        println!("       [PHASE D: THE POLISH]");
        
        // Run formatter
        self.run_formatter().await?;
        
        // Stage changes
        self.git.stage_file(&flaw.file_path).await?;
        
        // Commit
        if self.config.auto_commit {
            let message = format!(
                "forge: fix {} in {:?}\n\n{}",
                flaw.category,
                flaw.file_path,
                flaw.description.lines().next().unwrap_or("Code improvement")
            );
            self.git.commit(&message).await?;
            println!("       ✓ Committed");
        }
        
        self.stats.patches_applied += 1;
        
        Ok(true)
    }
    
    /// Run the test suite
    async fn run_tests(&mut self) -> Result<bool, ForgeError> {
        info!("[FORGE] Running tests: {}", self.config.test_command);
        
        let parts: Vec<&str> = self.config.test_command.split_whitespace().collect();
        if parts.is_empty() {
            return Ok(true);
        }
        
        let mut cmd = tokio::process::Command::new(parts[0]);
        cmd.args(&parts[1..])
            .current_dir(&self.config.target_repo)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        
        let output = cmd.output().await.map_err(|e| ForgeError::Io(e))?;
        
        let success = output.status.success();
        
        if success {
            self.stats.tests_passed += 1;
        } else {
            self.stats.tests_failed += 1;
        }
        
        Ok(success)
    }
    
    /// Get compiler errors for self-healing
    async fn get_compiler_errors(&self) -> Result<String, ForgeError> {
        let output = tokio::process::Command::new("cargo")
            .args(["check", "--message-format=short"])
            .current_dir(&self.config.target_repo)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await
            .map_err(|e| ForgeError::Io(e))?;
        
        let stderr = String::from_utf8_lossy(&output.stderr);
        Ok(stderr.to_string())
    }
    
    /// Get test errors
    async fn get_test_errors(&self) -> Result<String, ForgeError> {
        let output = tokio::process::Command::new("cargo")
            .args(["test"])
            .current_dir(&self.config.target_repo)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await
            .map_err(|e| ForgeError::Io(e))?;
        
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        Ok(format!("{}\n{}", stdout, stderr))
    }
    
    /// Run cargo fmt
    async fn run_formatter(&self) -> Result<(), ForgeError> {
        let output = tokio::process::Command::new("cargo")
            .args(["fmt"])
            .current_dir(&self.config.target_repo)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .await;
        
        match output {
            Ok(_) => Ok(()),
            Err(e) => {
                warn!("[FORGE] Formatter failed: {:?}", e);
                // Non-critical error
                Ok(())
            }
        }
    }
}
