use std::env;

#[derive(Debug, Clone, Default)]
pub struct LaunchIntent {
    workspace_path: Option<String>,
}

impl LaunchIntent {
    pub fn from_env_args() -> Self {
        let workspace_path = env::args()
            .nth(1)
            .map(|path| path.trim().to_string())
            .filter(|path| !path.is_empty());

        Self { workspace_path }
    }

    pub fn workspace_path(&self) -> Option<&str> {
        self.workspace_path.as_deref()
    }
}
