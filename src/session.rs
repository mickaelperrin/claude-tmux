use std::path::PathBuf;

/// Status of a Claude Code instance in a pane
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ClaudeCodeStatus {
    /// Waiting at prompt, ready for input
    Idle,
    /// Actively processing a request
    Working,
    /// Awaiting user confirmation/input (y/n prompt, etc.)
    WaitingInput,
    /// Cannot determine status
    #[default]
    Unknown,
}

impl ClaudeCodeStatus {
    /// Returns the display symbol for this status
    pub fn symbol(&self) -> &'static str {
        match self {
            ClaudeCodeStatus::Idle => "○",
            ClaudeCodeStatus::Working => "●",
            ClaudeCodeStatus::WaitingInput => "◐",
            ClaudeCodeStatus::Unknown => "?",
        }
    }

    /// Returns the display label for this status
    pub fn label(&self) -> &'static str {
        match self {
            ClaudeCodeStatus::Idle => "idle",
            ClaudeCodeStatus::Working => "working",
            ClaudeCodeStatus::WaitingInput => "input",
            ClaudeCodeStatus::Unknown => "unknown",
        }
    }
}

/// A tmux pane within a session
#[derive(Debug, Clone)]
pub struct Pane {
    /// Pane ID (e.g., "%0")
    pub id: String,
    /// Current command running in the pane
    pub current_command: String,
    /// Current working directory
    pub current_path: PathBuf,
}

/// A tmux session that may contain a Claude Code instance
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Session {
    /// Session name
    pub name: String,
    /// Unix timestamp when session was created
    pub created: i64,
    /// Whether a client is attached to this session
    pub attached: bool,
    /// Working directory (from the Claude Code pane, or first pane)
    pub working_directory: PathBuf,
    /// All panes in this session
    pub panes: Vec<Pane>,
    /// Pane ID containing Claude Code, if any
    pub claude_code_pane: Option<String>,
    /// Status of Claude Code in this session
    pub claude_code_status: ClaudeCodeStatus,
}

impl Session {
    /// Returns a shortened version of the working directory for display
    pub fn display_path(&self) -> String {
        let path = &self.working_directory;

        // Try to replace home directory with ~
        if let Some(home) = dirs::home_dir() {
            if let Ok(stripped) = path.strip_prefix(&home) {
                return format!("~/{}", stripped.display());
            }
        }

        path.display().to_string()
    }
}
