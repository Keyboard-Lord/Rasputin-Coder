use tracing::debug;

/// Canonical command set for operator shell
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    // Canonical repository commands
    OpenRepoPicker,
    OpenRepo {
        path: String,
    },
    SwitchRepo {
        path_or_name: String,
    },
    ArchiveConversation {
        id: String,
    },
    UnarchiveConversation {
        id: String,
    },
    DeleteProject {
        path_or_name: String,
    },

    // Canonical info commands
    ShowModel,
    ShowModels,
    SetModel {
        model: String,
    },
    ShowStatus,
    ShowHelp,
    RefreshRuntime,
    GitStatus,

    // Action commands
    RunValidation,
    ResetValidation,
    ClearLogs,
    RunForgeTask {
        task: String,
    },
    ReadFile {
        path: String,
    },
    WriteFile {
        path: String,
        content: String,
    },
    ReplaceInFile {
        path: String,
        find: String,
        replace: String,
        expected_hash: Option<String>,
    },
    RunShell {
        command: String,
    },
    ApprovePending,
    DenyPending,

    // General
    Quit,
    Unknown {
        input: String,
    },

    // Natural language task
    Task {
        content: String,
    },

    // RLEF management
    RLEFStatus,
    RLEFClear,
    RLEFDisableHint {
        class: String,
        guidance: String,
    },

    // Chain management
    ListChains,
    ChainStatus {
        chain_id: Option<String>,
    },
    ChainSwitch {
        chain_id: String,
    },
    ChainArchive {
        chain_id: String,
    },
    ChainResume {
        chain_id: String,
        force: bool,
    },
    // Replay/audit commands
    Replay {
        chain_id: Option<String>,
        replay_type: ReplayType,
    },
    Audit {
        chain_id: Option<String>,
    },
    // V1.6: Checkpoint management commands
    CheckpointList {
        chain_id: Option<String>,
    },
    CheckpointStatus {
        chain_id: Option<String>,
    },
    CheckpointShow {
        chain_id: String,
        checkpoint_id: String,
    },
    CheckpointDelete {
        chain_id: String,
        checkpoint_id: String,
    },
    // V1.6: Recovery status command
    ShowRecovery {
        chain_id: Option<String>,
    },

    // Plan review
    ShowPlan,
    ShowPlanContext,
    ShowPlanCheckpoints,

    // V1.3: Flow mode control
    FlowMode {
        enabled: bool,
    },

    // V1.3: Interrupt and override
    Stop,
    Cancel,
    Override,

    // V1.4: Multi-step lookahead
    Preview,

    // V2.0: Goal-driven autonomous operator
    Goal {
        statement: String,
    },
    GoalConfirm,
    GoalReject,
    GoalStatus,

    // V2.4: Project management
    ProjectCreate {
        name: String,
        path: Option<String>,
        init_git: bool,
    },
    ProjectSwitch {
        project_id: String,
    },
    ProjectList,
    DebugMode {
        enabled: bool,
    },

    // V2.5: Documentation generation - chain-aware multi-doc generation
    DocGenerate {
        repo_path: Option<String>,
        output_dir: Option<String>,
        doc_number: Option<u8>, // None = all 15, Some(n) = generate specific doc
    },
    DocGenerateChain {
        repo_path: String,
        output_dir: String,
        current_step: usize, // 1-15 for tracking progress
    },
    DocValidate {
        output_dir: String,
    },
    DocStatus,

    // V2.5: Auto-chain large prompts
    AutoChain {
        prompt: String,
        strategy: AutoChainStrategy,
    },
    
    // V2.6: Large prompt decomposer with artifact contract
    ArtifactContract {
        prompt: String,
        auto_detect: bool,
    },
}

/// Strategy for auto-chaining large prompts
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AutoChainStrategy {
    /// Auto-detect based on content
    Auto,
    /// Split by document boundaries
    ByDocument,
    /// Split by file operations
    ByFile,
    /// Split by paragraphs/sections
    BySection,
    /// Chunk by token size
    BySize { max_tokens: usize },
}

/// Type of replay operation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayType {
    /// Replay a chain
    Chain,
    /// Replay a specific step
    Step,
    /// Show replay status
    Status,
    /// Show replay diff/comparison
    Diff,
    /// V1.6: Replay from audit log (deterministic reconstruction)
    Audit,
}

pub fn parse_command(text: &str) -> Command {
    let trimmed = text.trim();
    let lower = trimmed.to_lowercase();

    debug!("Parsing command: {}", trimmed);

    // App-native: /open invokes the host folder picker
    if lower == "/open" || lower == "/project open" || lower == "/project connect" {
        return Command::OpenRepoPicker;
    }

    // Canonical: /open <path>
    if lower.starts_with("/open ") {
        let path = trimmed.trim_start_matches("/open ").trim().to_string();
        return Command::OpenRepo { path };
    }

    // Canonical: /switch <path or recent repo>
    if lower.starts_with("/switch ") {
        let path_or_name = trimmed.trim_start_matches("/switch ").trim().to_string();
        return Command::SwitchRepo { path_or_name };
    }

    if lower.starts_with("/project delete ") {
        let path_or_name = trimmed
            .trim_start_matches("/project delete ")
            .trim()
            .to_string();
        return Command::DeleteProject { path_or_name };
    }

    // V2.4: /project create <name> [--path <path>] [--git]
    if lower.starts_with("/project create ") {
        let args = trimmed.trim_start_matches("/project create ").trim();

        // Parse arguments
        let mut name = None;
        let mut path = None;
        let mut init_git = false;

        // Simple argument parsing
        let parts: Vec<&str> = args.split_whitespace().collect();
        let mut i = 0;

        while i < parts.len() {
            match parts[i] {
                "--path" | "-p" => {
                    if i + 1 < parts.len() {
                        path = Some(parts[i + 1].to_string());
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                "--git" | "-g" => {
                    init_git = true;
                    i += 1;
                }
                _ => {
                    // First non-flag argument is the name
                    if name.is_none() && !parts[i].starts_with("-") {
                        name = Some(parts[i].to_string());
                    }
                    i += 1;
                }
            }
        }

        if let Some(name) = name {
            return Command::ProjectCreate {
                name,
                path,
                init_git,
            };
        }
    }

    // V2.4: /project switch <project_id>
    if lower.starts_with("/project switch ") {
        let project_id = trimmed
            .trim_start_matches("/project switch ")
            .trim()
            .to_string();
        return Command::ProjectSwitch { project_id };
    }

    // V2.4: /project list
    if lower == "/project list" || lower == "/projects" {
        return Command::ProjectList;
    }

    if lower == "/debug" || lower == "/operator" || lower == "/debug on" {
        return Command::DebugMode { enabled: true };
    }

    if lower == "/debug off" || lower == "/normal" {
        return Command::DebugMode { enabled: false };
    }

    // Canonical: /model (show active model)
    if lower == "/model" {
        return Command::ShowModel;
    }

    if lower.starts_with("/model use ") {
        let model = trimmed.trim_start_matches("/model use ").trim().to_string();
        return Command::SetModel { model };
    }

    if lower.starts_with("/model set ") {
        let model = trimmed.trim_start_matches("/model set ").trim().to_string();
        return Command::SetModel { model };
    }

    if lower.starts_with("/model ") {
        let model = trimmed.trim_start_matches("/model ").trim().to_string();
        return Command::SetModel { model };
    }

    if lower.starts_with("/config set planner_model ") {
        let model = trimmed
            .trim_start_matches("/config set planner_model ")
            .trim()
            .to_string();
        return Command::SetModel { model };
    }

    // Canonical: /models (show installed Ollama models)
    if lower == "/models" {
        return Command::ShowModels;
    }

    // Canonical: /status (show runtime status)
    if lower == "/status" {
        return Command::ShowStatus;
    }

    // Canonical: /git or /git status (show Git repository status)
    if lower == "/git" || lower == "/git status" {
        return Command::GitStatus;
    }

    // Canonical: /validate (run validation pipeline)
    if lower == "/validate" || lower == "/v" {
        return Command::RunValidation;
    }

    // Legacy/manual: /task <description> (run Forge execution)
    if lower.starts_with("/task ") {
        let task = trimmed.trim_start_matches("/task ").trim().to_string();
        return Command::RunForgeTask { task };
    }

    if lower.starts_with("/read ") {
        let path = trimmed.trim_start_matches("/read ").trim().to_string();
        return Command::ReadFile { path };
    }

    if lower.starts_with("/write ") {
        let body = trimmed.trim_start_matches("/write ").trim();
        if let Some((path, content)) = body.split_once(" -- ") {
            return Command::WriteFile {
                path: path.trim().to_string(),
                content: content.replace("\\n", "\n"),
            };
        }
    }

    if lower.starts_with("/replace ") {
        let body = trimmed.trim_start_matches("/replace ").trim();
        if let Some((path, rest)) = body.split_once(" --find ")
            && let Some((find, rest2)) = rest.split_once(" --replace ")
        {
            // Check for optional --hash parameter
            let (replace_part, hash) = if let Some((rep, hash_part)) = rest2.split_once(" --hash ")
            {
                (rep.trim(), Some(hash_part.trim().to_string()))
            } else {
                (rest2.trim(), None)
            };
            return Command::ReplaceInFile {
                path: path.trim().to_string(),
                find: find.replace("\\n", "\n"),
                replace: replace_part.replace("\\n", "\n"),
                expected_hash: hash,
            };
        }
    }

    if lower.starts_with("/run ") {
        let command = trimmed.trim_start_matches("/run ").trim().to_string();
        return Command::RunShell { command };
    }

    if lower == "/approve" {
        return Command::ApprovePending;
    }

    if lower == "/deny" {
        return Command::DenyPending;
    }

    // Canonical: /help
    if lower == "/help" || lower == "help" || lower == "/?" {
        return Command::ShowHelp;
    }

    // Canonical: /quit
    if lower == "/quit" || lower == "/exit" {
        return Command::Quit;
    }

    // RLEF management commands
    if lower == "/rlef status" {
        return Command::RLEFStatus;
    }

    if lower == "/rlef clear" {
        return Command::RLEFClear;
    }

    if lower.starts_with("/rlef disable ") {
        let rest = trimmed.trim_start_matches("/rlef disable ").trim();
        // Parse format: <class> -- <guidance>
        if let Some(pos) = rest.find(" -- ") {
            let class = rest[..pos].to_string();
            let guidance = rest[pos + 4..].to_string();
            return Command::RLEFDisableHint { class, guidance };
        }
    }

    // Chain management commands
    if lower == "/chains" {
        return Command::ListChains;
    }

    if lower == "/chain status" {
        return Command::ChainStatus { chain_id: None };
    }

    if lower.starts_with("/chain status ") {
        let chain_id = trimmed
            .trim_start_matches("/chain status ")
            .trim()
            .to_string();
        return Command::ChainStatus {
            chain_id: Some(chain_id),
        };
    }

    if lower.starts_with("/chain switch ") {
        let chain_id = trimmed
            .trim_start_matches("/chain switch ")
            .trim()
            .to_string();
        return Command::ChainSwitch { chain_id };
    }

    if lower.starts_with("/chain archive ") {
        let chain_id = trimmed
            .trim_start_matches("/chain archive ")
            .trim()
            .to_string();
        return Command::ChainArchive { chain_id };
    }

    if lower.starts_with("/chain resume ") {
        let args = trimmed.trim_start_matches("/chain resume ").trim();
        // Check for --force flag
        let (chain_id, force) = if args.contains("--force") {
            let id = args.replace("--force", "").trim().to_string();
            (id, true)
        } else {
            (args.to_string(), false)
        };
        return Command::ChainResume { chain_id, force };
    }

    // Replay/audit commands
    if lower == "/replay" || lower == "/replay active" {
        return Command::Replay {
            chain_id: None, // Use active chain
            replay_type: ReplayType::Chain,
        };
    }

    if lower == "/replay status" {
        return Command::Replay {
            chain_id: None,
            replay_type: ReplayType::Status,
        };
    }

    if lower == "/replay diff" {
        return Command::Replay {
            chain_id: None,
            replay_type: ReplayType::Diff,
        };
    }

    if lower.starts_with("/replay ") {
        let rest = trimmed.trim_start_matches("/replay ").trim();
        // Check if it's a chain ID or has subcommand
        if rest == "active" {
            return Command::Replay {
                chain_id: None,
                replay_type: ReplayType::Chain,
            };
        } else if rest == "status" {
            return Command::Replay {
                chain_id: None,
                replay_type: ReplayType::Status,
            };
        } else if rest == "diff" {
            return Command::Replay {
                chain_id: None,
                replay_type: ReplayType::Diff,
            };
        } else {
            // Treat as chain ID
            return Command::Replay {
                chain_id: Some(rest.to_string()),
                replay_type: ReplayType::Chain,
            };
        }
    }

    // V1.6 AUDIT: Audit timeline commands
    if lower == "/audit" || lower == "/audit active" {
        return Command::Audit {
            chain_id: None, // Use active chain
        };
    }

    if lower.starts_with("/audit ") {
        let rest = trimmed.trim_start_matches("/audit ").trim();
        return Command::Audit {
            chain_id: Some(rest.to_string()),
        };
    }

    // V1.6 AUDIT: Chain audit subcommand
    if lower == "/chain audit" {
        return Command::Audit {
            chain_id: None, // Use active chain
        };
    }

    if lower.starts_with("/chain audit ") {
        let chain_id = trimmed
            .trim_start_matches("/chain audit ")
            .trim()
            .to_string();
        return Command::Audit {
            chain_id: Some(chain_id),
        };
    }

    // V1.6 REPLAY: Audit replay commands
    if lower == "/audit replay" || lower == "/replay audit" {
        return Command::Replay {
            chain_id: None,
            replay_type: ReplayType::Audit,
        };
    }

    if lower.starts_with("/audit replay ") {
        let chain_id = trimmed
            .trim_start_matches("/audit replay ")
            .trim()
            .to_string();
        return Command::Replay {
            chain_id: Some(chain_id),
            replay_type: ReplayType::Audit,
        };
    }

    if lower == "/chain replay" {
        return Command::Replay {
            chain_id: None,
            replay_type: ReplayType::Audit,
        };
    }

    if lower.starts_with("/chain replay ") {
        let chain_id = trimmed
            .trim_start_matches("/chain replay ")
            .trim()
            .to_string();
        return Command::Replay {
            chain_id: Some(chain_id),
            replay_type: ReplayType::Audit,
        };
    }

    // V1.6: Checkpoint management commands
    if lower == "/checkpoint list" || lower == "/checkpoints" {
        return Command::CheckpointList { chain_id: None };
    }

    if lower.starts_with("/checkpoint list ") {
        let chain_id = trimmed
            .trim_start_matches("/checkpoint list ")
            .trim()
            .to_string();
        return Command::CheckpointList {
            chain_id: Some(chain_id),
        };
    }

    if lower == "/chain checkpoints" {
        return Command::CheckpointList { chain_id: None };
    }

    if lower.starts_with("/chain checkpoint ") {
        let rest = trimmed.trim_start_matches("/chain checkpoint ").trim();
        let parts: Vec<&str> = rest.split_whitespace().collect();
        if parts.len() >= 2 {
            return Command::CheckpointShow {
                chain_id: parts[0].to_string(),
                checkpoint_id: parts[1].to_string(),
            };
        }
    }

    // Plan review commands
    if lower == "/plan" {
        return Command::ShowPlan;
    }

    if lower == "/plan context" {
        return Command::ShowPlanContext;
    }

    if lower == "/plan checkpoints" {
        return Command::ShowPlanCheckpoints;
    }

    if lower == "/checkpoint" || lower == "/checkpoint status" {
        return Command::CheckpointStatus { chain_id: None };
    }

    if lower.starts_with("/checkpoint status ") {
        let chain_id = trimmed.trim_start_matches("/checkpoint status ").trim();
        return Command::CheckpointStatus {
            chain_id: Some(chain_id.to_string()),
        };
    }

    // V1.6: Recovery status commands
    if lower == "/recovery" || lower == "/recovery active" {
        return Command::ShowRecovery {
            chain_id: None, // Use active chain
        };
    }

    if lower.starts_with("/recovery ") {
        let rest = trimmed.trim_start_matches("/recovery ").trim();
        return Command::ShowRecovery {
            chain_id: Some(rest.to_string()),
        };
    }

    if lower == "/chain recovery" {
        return Command::ShowRecovery {
            chain_id: None, // Use active chain
        };
    }

    // V1.3: Flow mode control
    if lower == "/flow on" || lower == "/flow enable" {
        return Command::FlowMode { enabled: true };
    }

    if lower == "/flow off" || lower == "/flow disable" {
        return Command::FlowMode { enabled: false };
    }

    if lower == "/flow" {
        // Toggle flow mode
        return Command::FlowMode { enabled: true };
    }

    // V1.3: Interrupt commands
    if lower == "/stop" || lower == "/halt" {
        return Command::Stop;
    }

    if lower == "/cancel" || lower == "/abort" {
        return Command::Cancel;
    }

    if lower == "/override" {
        return Command::Override;
    }

    // V1.4: Preview upcoming execution
    if lower == "/preview" {
        return Command::Preview;
    }

    if lower == "/goal confirm" || lower == "/goal ok" {
        return Command::GoalConfirm;
    }

    if lower == "/goal reject" || lower == "/goal cancel" {
        return Command::GoalReject;
    }

    if lower == "/goal status" || lower == "/goal" {
        return Command::GoalStatus;
    }

    // V2.0: Goal-driven autonomous operator
    if lower.starts_with("/goal ") {
        let statement = trimmed.trim_start_matches("/goal ").trim().to_string();
        if !statement.is_empty() {
            return Command::Goal { statement };
        }
    }

    // Resume/continue aliases
    if lower == "/resume" || lower == "/continue" {
        // Without explicit chain_id, use active chain
        return Command::ChainResume {
            chain_id: "active".to_string(),
            force: false,
        };
    }

    if lower.starts_with("/resume ") {
        let args = trimmed.trim_start_matches("/resume ").trim();
        let (chain_id, force) = if args.contains("--force") {
            let id = args.replace("--force", "").trim().to_string();
            (id, true)
        } else {
            (args.to_string(), false)
        };
        return Command::ChainResume { chain_id, force };
    }

    // V2.5: Documentation generation commands
    if lower == "/doc generate" || lower == "/docs generate" {
        return Command::DocGenerate {
            repo_path: None,
            output_dir: None,
            doc_number: None,
        };
    }

    if lower.starts_with("/doc generate ") || lower.starts_with("/docs generate ") {
        let args = trimmed.trim_start_matches("/doc generate ")
            .trim_start_matches("/docs generate ")
            .trim();
        
        let mut repo_path = None;
        let mut output_dir = None;
        let mut doc_number = None;
        
        // Parse arguments: --repo <path> --out <dir> --doc <n>
        let parts: Vec<&str> = args.split_whitespace().collect();
        let mut i = 0;
        while i < parts.len() {
            match parts[i] {
                "--repo" | "-r" => {
                    if i + 1 < parts.len() {
                        repo_path = Some(parts[i + 1].to_string());
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                "--out" | "-o" => {
                    if i + 1 < parts.len() {
                        output_dir = Some(parts[i + 1].to_string());
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                "--doc" | "-d" => {
                    if i + 1 < parts.len() {
                        if let Ok(n) = parts[i + 1].parse::<u8>() {
                            doc_number = Some(n);
                        }
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                _ => i += 1,
            }
        }
        
        return Command::DocGenerate {
            repo_path,
            output_dir,
            doc_number,
        };
    }

    if lower == "/doc status" || lower == "/docs status" {
        return Command::DocStatus;
    }

    if lower.starts_with("/doc validate ") {
        let output_dir = trimmed.trim_start_matches("/doc validate ").trim().to_string();
        return Command::DocValidate { output_dir };
    }

    // V2.5: Auto-chain command for large prompts
    if lower.starts_with("/auto-chain ") || lower.starts_with("/chain-auto ") {
        let prompt = trimmed
            .trim_start_matches("/auto-chain ")
            .trim_start_matches("/chain-auto ")
            .trim()
            .to_string();
        
        // Detect strategy from prompt content
        let strategy = detect_chain_strategy(&prompt);
        
        return Command::AutoChain { prompt, strategy };
    }

    // V2.6: Artifact contract command for large prompts with explicit deliverables
    if lower.starts_with("/artifact-contract ") || lower.starts_with("/contract ") {
        let prompt = trimmed
            .trim_start_matches("/artifact-contract ")
            .trim_start_matches("/contract ")
            .trim()
            .to_string();
        
        return Command::ArtifactContract { prompt, auto_detect: true };
    }

    if trimmed.starts_with('/') {
        return Command::Unknown {
            input: trimmed.to_string(),
        };
    }

    // Default: natural language task
    Command::Task {
        content: trimmed.to_string(),
    }
}

pub fn get_help_text() -> &'static str {
    r#"Rasputin Operator Shell - Canonical Commands

Repository:
  /open <path>              Attach a local repo/folder
  /switch <path|name>       Switch to repo (by path or recent name)
  /project delete <path>    Delete a project folder (approval required)

Info:
  /model                    Show active model configuration
  /models                   Show installed Ollama models
  /model <tag>              Set the repo planner model (for example: /model 14b)
  /config set planner_model <tag>
                            Same as /model <tag>
  /status                   Show runtime status and connection state
  /validate                 Run validation pipeline (syntax, lint, build, test)
  /goal <description>       Plan a bounded autonomous goal with Qwen-Coder
  /goal confirm             Accept the plan and start the bounded chain
  /task <description>       Legacy/manual Forge task entrypoint
  /read <path>              Read a file inside the active project
  /write <path> -- <text>   Write file contents inside the active project
  /replace <path> --find <text> --replace <text>
                            Replace text inside a project file
  /run <command>            Run a shell command in the active project
  /approve                  Approve the pending destructive/command action
  /deny                     Cancel the pending destructive/command action
  /help                     Show this help

Chain Management:
  /chains                   List all chains
  /chain status [id]        Show chain status (active or specified)
  /chain switch <id>        Switch to a different chain
  /chain archive <id>       Archive a completed/failed chain
  /chain resume <id>        Resume a halted chain
  /resume                   Resume active chain (alias: /continue)

Plan Review:
  /plan                     Show plan summary for active chain
  /plan context             Show selected context files
  /plan checkpoints         Show expected checkpoints
  /checkpoint               Show checkpoint status
  /checkpoint status        Show checkpoint status
  /recovery                 Show recovery/self-healing status
  /recovery [chain_id]      Show recovery status for specific chain

RLEF (Learning):
  /rlef status              Show execution feedback statistics
  /rlef clear               Clear all learned hints
  /rlef disable <class> -- <guidance>
                            Disable a specific hint

Documentation Generation (V2.5):
  /doc generate             Generate all 15 canonical documentation files
  /doc generate --doc <n>   Generate specific document (1-15)
  /doc generate --repo <path> --out <dir>
                            Generate docs for specific repo
  /doc status               Show documentation generation status
  /doc validate <dir>       Validate generated documentation

Large Prompt Decomposition (V2.6):
  /artifact-contract <prompt>  Decompose large prompt into bounded chain
  /contract <prompt>           Alias for /artifact-contract

General:
  /quit                     Exit Rasputin (also Ctrl+C)

Navigation:
  Enter                     Submit composer text
  Esc                       Leave composer editing mode
  i                         Re-enter composer from navigation mode
  Tab (editing)             Toggle inspector visibility
  Tab / Shift+Tab           Cycle focus in navigation mode
  Enter (navigation)        Activate focused control
  Mouse                     Click sidebar, tabs, and panel controls

Interaction truth:
  Task-like plain text      Plans a goal and queues bounded autonomous execution
  Question-like plain chat  Talks to Ollama only
  EDIT mode                 Enables real file reads/writes
  /goal <description>       Uses Qwen-Coder first, with heuristic fallback
  /goal confirm             Enables auto-resume/auto-advance within policy gates
  /task <description>       Legacy manual Forge task path
  /validate                 Runs the local validation pipeline
  Follow-up questions       Do NOT continue the previous Forge run
  Unknown slash commands    Fail explicitly and do not become chat or task text

Read the inspector for task progress. The main chat pane only shows compact task notices."#
}

/// Detect the optimal chain strategy for a large prompt
fn detect_chain_strategy(prompt: &str) -> AutoChainStrategy {
    let lower = prompt.to_lowercase();
    
    // Check for 15 canonical docs pattern
    if lower.contains("canonical") && lower.contains("doc")
        || lower.matches(".md").count() >= 5
        || lower.matches("## ").count() >= 10 {
        return AutoChainStrategy::ByDocument;
    }
    
    // Check for multiple file operations
    if lower.matches("write_file").count() >= 3
        || lower.matches("create file").count() >= 3
        || lower.matches("generate file").count() >= 3 {
        return AutoChainStrategy::ByFile;
    }
    
    // Check for section-based content
    if lower.matches("\n## ").count() >= 5
        || lower.matches("\n### ").count() >= 5 {
        return AutoChainStrategy::BySection;
    }
    
    // Default: auto-detect at runtime
    AutoChainStrategy::Auto
}

#[cfg(test)]
mod tests {
    use super::{Command, parse_command};

    #[test]
    fn exact_goal_subcommands_win_before_goal_text() {
        assert_eq!(parse_command("/goal confirm"), Command::GoalConfirm);
        assert_eq!(parse_command("/goal ok"), Command::GoalConfirm);
        assert_eq!(parse_command("/goal reject"), Command::GoalReject);
        assert_eq!(parse_command("/goal cancel"), Command::GoalReject);
        assert_eq!(parse_command("/goal status"), Command::GoalStatus);
        assert_eq!(parse_command("/goal"), Command::GoalStatus);

        assert_eq!(
            parse_command("/goal harden parser routing"),
            Command::Goal {
                statement: "harden parser routing".to_string()
            }
        );
    }

    #[test]
    fn unknown_slash_commands_fail_explicitly() {
        assert_eq!(
            parse_command("/writ src/main.rs"),
            Command::Unknown {
                input: "/writ src/main.rs".to_string()
            }
        );
    }

    #[test]
    fn malformed_known_slash_commands_do_not_become_tasks() {
        assert_eq!(
            parse_command("/write src/main.rs"),
            Command::Unknown {
                input: "/write src/main.rs".to_string()
            }
        );
    }

    #[test]
    fn plain_text_still_routes_to_task() {
        assert_eq!(
            parse_command("explain the project"),
            Command::Task {
                content: "explain the project".to_string()
            }
        );
    }

    #[test]
    fn documented_checkpoint_aliases_route_to_checkpoint_status() {
        assert_eq!(
            parse_command("/checkpoint"),
            Command::CheckpointStatus { chain_id: None }
        );
        assert_eq!(
            parse_command("/checkpoint status"),
            Command::CheckpointStatus { chain_id: None }
        );
    }

    #[test]
    fn documented_recovery_aliases_route_to_show_recovery() {
        assert_eq!(
            parse_command("/recovery"),
            Command::ShowRecovery { chain_id: None }
        );
        assert_eq!(
            parse_command("/recovery active"),
            Command::ShowRecovery { chain_id: None }
        );
        assert_eq!(
            parse_command("/recovery my-chain-id"),
            Command::ShowRecovery {
                chain_id: Some("my-chain-id".to_string())
            }
        );
        assert_eq!(
            parse_command("/chain recovery"),
            Command::ShowRecovery { chain_id: None }
        );
    }

    #[test]
    fn app_native_open_aliases_route_to_folder_picker() {
        assert_eq!(parse_command("/open"), Command::OpenRepoPicker);
        assert_eq!(parse_command("/project open"), Command::OpenRepoPicker);
        assert_eq!(parse_command("/project connect"), Command::OpenRepoPicker);
    }

    #[test]
    fn debug_mode_aliases_are_explicit() {
        assert_eq!(
            parse_command("/debug"),
            Command::DebugMode { enabled: true }
        );
        assert_eq!(
            parse_command("/operator"),
            Command::DebugMode { enabled: true }
        );
        assert_eq!(
            parse_command("/debug off"),
            Command::DebugMode { enabled: false }
        );
        assert_eq!(
            parse_command("/normal"),
            Command::DebugMode { enabled: false }
        );
    }
}
