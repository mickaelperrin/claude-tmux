use std::path::PathBuf;

use crate::git::GitContext;

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
    /// Pane index within the window
    pub pane_index: usize,
    /// Process ID of the pane's shell
    pub pid: u32,
    /// Current working directory
    pub current_path: PathBuf,
    /// Window index
    pub window_index: usize,
    /// Window name
    pub window_name: String,
}

/// A Claude Code instance running in a tmux pane
#[derive(Debug, Clone)]
pub struct ClaudeInstance {
    // Session info
    /// Session name
    pub session_name: String,
    /// Whether a client is attached to this session
    pub session_attached: bool,

    // Window info
    /// Window index within the session
    pub window_index: usize,
    /// Window name
    pub window_name: String,

    // Pane info
    /// Pane ID (e.g., "%0")
    pub pane_id: String,
    /// Pane index within the window
    pub pane_index: usize,

    // Claude info
    /// Working directory of the pane
    pub working_directory: PathBuf,
    /// Status of Claude Code
    pub status: ClaudeCodeStatus,
    /// Git context, if the working directory is a git repository
    pub git_context: Option<GitContext>,
}

impl ClaudeInstance {
    /// Returns a display name combining session, window, and pane info
    pub fn display_name(&self) -> String {
        format!(
            "{}:{}.{}",
            self.session_name, self.window_index, self.pane_index
        )
    }

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

    /// Returns the tmux target for this pane (for switch-client, send-keys, etc.)
    pub fn tmux_target(&self) -> String {
        format!(
            "{}:{}.{}",
            self.session_name, self.window_index, self.pane_index
        )
    }
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
    /// Number of windows in this session
    pub window_count: usize,
    /// All panes in this session
    pub panes: Vec<Pane>,
    /// Pane ID containing Claude Code, if any
    pub claude_code_pane: Option<String>,
    /// Status of Claude Code in this session
    pub claude_code_status: ClaudeCodeStatus,
    /// Git context, if the working directory is a git repository
    pub git_context: Option<GitContext>,
}
