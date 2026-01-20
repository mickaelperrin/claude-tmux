use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Command;

use anyhow::{Context, Result};

use crate::detection::detect_status;
use crate::git::GitContext;
use crate::session::{ClaudeCodeStatus, ClaudeInstance, Pane};

/// Wrapper for tmux command execution
pub struct Tmux;

impl Tmux {
    /// List all Claude Code instances across all tmux sessions
    pub fn list_claude_instances() -> Result<Vec<ClaudeInstance>> {
        // Get list of sessions
        let output = Command::new("tmux")
            .args([
                "list-sessions",
                "-F",
                "#{session_name}\t#{session_attached}",
            ])
            .output()
            .context("Failed to execute tmux list-sessions")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("no server running") || stderr.contains("no sessions") {
                return Ok(Vec::new());
            }
            anyhow::bail!("tmux list-sessions failed: {}", stderr);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);

        // Collect all panes from all sessions
        let mut all_panes: Vec<(String, bool, Pane)> = Vec::new(); // (session_name, attached, pane)

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 2 {
                let session_name = parts[0].to_string();
                let attached = parts[1] == "1";

                // Get all panes for this session
                let panes = Self::list_panes(&session_name).unwrap_or_default();
                for pane in panes {
                    all_panes.push((session_name.clone(), attached, pane));
                }
            }
        }

        // Collect all pane PIDs
        let all_pane_pids: Vec<u32> = all_panes
            .iter()
            .filter(|(_, _, pane)| pane.pid > 0)
            .map(|(_, _, pane)| pane.pid)
            .collect();

        // Find which panes have Claude Code running
        let panes_with_claude = Self::find_panes_with_claude(&all_pane_pids);

        // Build ClaudeInstance for each pane with Claude
        let mut instances: Vec<ClaudeInstance> = Vec::new();

        for (session_name, attached, pane) in all_panes {
            if panes_with_claude.contains(&pane.pid) {
                // Detect Claude status
                let status = Self::capture_pane(&pane.id, 15, true)
                    .map(|content| detect_status(&content))
                    .unwrap_or(ClaudeCodeStatus::Unknown);

                // Detect git context
                let git_context = GitContext::detect(&pane.current_path);

                instances.push(ClaudeInstance {
                    session_name,
                    session_attached: attached,
                    window_index: pane.window_index,
                    window_name: pane.window_name,
                    pane_id: pane.id,
                    pane_index: pane.pane_index,
                    working_directory: pane.current_path,
                    status,
                    git_context,
                });
            }
        }

        // Sort by: attached sessions first, then session name, then window index, then pane index
        instances.sort_by(|a, b| {
            b.session_attached
                .cmp(&a.session_attached)
                .then_with(|| a.session_name.cmp(&b.session_name))
                .then_with(|| a.window_index.cmp(&b.window_index))
                .then_with(|| a.pane_index.cmp(&b.pane_index))
        });

        Ok(instances)
    }

    /// List all panes in a session (across all windows)
    fn list_panes(session: &str) -> Result<Vec<Pane>> {
        let output = Command::new("tmux")
            .args([
                "list-panes",
                "-t",
                session,
                "-s", // List all panes in all windows
                "-F",
                "#{pane_id}\t#{pane_index}\t#{pane_pid}\t#{pane_current_path}\t#{window_index}\t#{window_name}",
            ])
            .output()
            .context("Failed to execute tmux list-panes")?;

        if !output.status.success() {
            return Ok(Vec::new());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut panes = Vec::new();

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 6 {
                panes.push(Pane {
                    id: parts[0].to_string(),
                    pane_index: parts[1].parse().unwrap_or(0),
                    pid: parts[2].parse().unwrap_or(0),
                    current_path: PathBuf::from(parts[3]),
                    window_index: parts[4].parse().unwrap_or(0),
                    window_name: parts[5].to_string(),
                });
            }
        }

        Ok(panes)
    }

    /// List all panes across all sessions in a single tmux call
    ///
    /// This is more efficient than calling list_panes() for each session separately.
    fn list_all_panes() -> Result<Vec<(String, bool, Pane)>> {
        let output = Command::new("tmux")
            .args([
                "list-panes",
                "-a", // All sessions, all windows
                "-F",
                "#{session_name}\t#{session_attached}\t#{pane_id}\t#{pane_index}\t#{pane_pid}\t#{pane_current_path}\t#{window_index}\t#{window_name}",
            ])
            .output()
            .context("Failed to execute tmux list-panes -a")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("no server running") || stderr.contains("no sessions") {
                return Ok(Vec::new());
            }
            anyhow::bail!("tmux list-panes -a failed: {}", stderr);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut all_panes = Vec::new();

        for line in stdout.lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() >= 8 {
                let session_name = parts[0].to_string();
                let attached = parts[1] == "1";
                let pane = Pane {
                    id: parts[2].to_string(),
                    pane_index: parts[3].parse().unwrap_or(0),
                    pid: parts[4].parse().unwrap_or(0),
                    current_path: PathBuf::from(parts[5]),
                    window_index: parts[6].parse().unwrap_or(0),
                    window_name: parts[7].to_string(),
                };
                all_panes.push((session_name, attached, pane));
            }
        }

        Ok(all_panes)
    }

    /// List Claude instances without git context (for fast initial loading)
    ///
    /// This uses batch tmux commands for efficiency. Git context should be
    /// loaded separately via GitContext::detect() in a background thread.
    pub fn list_claude_instances_basic() -> Result<Vec<ClaudeInstance>> {
        // Get all panes in a single tmux call
        let all_panes = Self::list_all_panes()?;

        // Collect all pane PIDs
        let all_pane_pids: Vec<u32> = all_panes
            .iter()
            .filter(|(_, _, pane)| pane.pid > 0)
            .map(|(_, _, pane)| pane.pid)
            .collect();

        // Find which panes have Claude Code running
        let panes_with_claude = Self::find_panes_with_claude(&all_pane_pids);

        // Build ClaudeInstance for each pane with Claude (without git context)
        let mut instances: Vec<ClaudeInstance> = Vec::new();

        for (session_name, attached, pane) in all_panes {
            if panes_with_claude.contains(&pane.pid) {
                // Detect Claude status
                let status = Self::capture_pane(&pane.id, 15, true)
                    .map(|content| detect_status(&content))
                    .unwrap_or(ClaudeCodeStatus::Unknown);

                instances.push(ClaudeInstance {
                    session_name,
                    session_attached: attached,
                    window_index: pane.window_index,
                    window_name: pane.window_name,
                    pane_id: pane.id,
                    pane_index: pane.pane_index,
                    working_directory: pane.current_path,
                    status,
                    git_context: None, // Will be loaded separately
                });
            }
        }

        // Sort by: attached sessions first, then session name, then window index, then pane index
        instances.sort_by(|a, b| {
            b.session_attached
                .cmp(&a.session_attached)
                .then_with(|| a.session_name.cmp(&b.session_name))
                .then_with(|| a.window_index.cmp(&b.window_index))
                .then_with(|| a.pane_index.cmp(&b.pane_index))
        });

        Ok(instances)
    }

    /// Get the process parent map (pid -> ppid) for all processes
    fn get_process_parent_map() -> HashMap<u32, u32> {
        let mut map = HashMap::new();

        let output = Command::new("ps").args(["-eo", "pid,ppid"]).output().ok();

        if let Some(output) = output {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines().skip(1) {
                    // Skip header
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 2 {
                        if let (Ok(pid), Ok(ppid)) =
                            (parts[0].parse::<u32>(), parts[1].parse::<u32>())
                        {
                            map.insert(pid, ppid);
                        }
                    }
                }
            }
        }

        map
    }

    /// Get PIDs of all running claude processes
    fn get_claude_pids() -> Vec<u32> {
        let mut pids = Vec::new();

        let output = Command::new("pgrep")
            .args(["-f", "bin/claude"])
            .output()
            .ok();

        if let Some(output) = output {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    if let Ok(pid) = line.trim().parse::<u32>() {
                        pids.push(pid);
                    }
                }
            }
        }

        pids
    }

    /// Find which pane_pids have a claude process as a descendant
    fn find_panes_with_claude(pane_pids: &[u32]) -> HashSet<u32> {
        let parent_map = Self::get_process_parent_map();
        let claude_pids = Self::get_claude_pids();
        let pane_pid_set: HashSet<u32> = pane_pids.iter().copied().collect();

        let mut panes_with_claude = HashSet::new();

        // For each claude process, walk up the parent chain to find if it's under a pane_pid
        for claude_pid in claude_pids {
            let mut current = claude_pid;
            let mut visited = HashSet::new();

            // Walk up the parent chain (max 100 iterations to prevent infinite loops)
            while current > 1 && visited.len() < 100 {
                if visited.contains(&current) {
                    break; // Cycle detected
                }
                visited.insert(current);

                if pane_pid_set.contains(&current) {
                    panes_with_claude.insert(current);
                    break;
                }

                // Move to parent
                if let Some(&ppid) = parent_map.get(&current) {
                    current = ppid;
                } else {
                    break;
                }
            }
        }

        panes_with_claude
    }

    /// Capture the last N lines of a pane's content
    ///
    /// If `strip_empty` is true, empty lines are filtered out before taking the last N.
    /// This is useful for status detection. For preview display, use `strip_empty: false`
    /// to preserve the visual layout.
    ///
    /// ANSI escape sequences are always included - the UI handles rendering them.
    pub fn capture_pane(pane_id: &str, lines: usize, strip_empty: bool) -> Result<String> {
        let output = Command::new("tmux")
            .args([
                "capture-pane",
                "-t",
                pane_id,
                "-p", // Print to stdout
                "-J", // Join wrapped lines
                "-e", // Include escape sequences
            ])
            .output()
            .context("Failed to capture pane")?;

        if !output.status.success() {
            anyhow::bail!("Failed to capture pane {}", pane_id);
        }

        let content = String::from_utf8_lossy(&output.stdout);

        if strip_empty {
            // Filter out empty lines, then get last N (for status detection)
            let non_empty: Vec<&str> = content.lines().filter(|l| !l.trim().is_empty()).collect();
            let start = non_empty.len().saturating_sub(lines);
            let last_lines = &non_empty[start..];
            Ok(last_lines.join("\n"))
        } else {
            // Preserve internal empty lines but trim trailing ones (for preview display)
            let all_lines: Vec<&str> = content.lines().collect();

            // Find last non-empty line
            let last_non_empty = all_lines
                .iter()
                .rposition(|l| !l.trim().is_empty())
                .map(|i| i + 1)
                .unwrap_or(0);

            let trimmed = &all_lines[..last_non_empty];
            let start = trimmed.len().saturating_sub(lines);
            let last_lines = &trimmed[start..];
            Ok(last_lines.join("\n"))
        }
    }

    /// Switch the current client to a specific pane (target format: session:window.pane)
    pub fn switch_to_pane(target: &str) -> Result<()> {
        let status = Command::new("tmux")
            .args(["switch-client", "-t", target])
            .status()
            .context("Failed to switch to pane")?;

        if !status.success() {
            anyhow::bail!("Failed to switch to pane {}", target);
        }

        Ok(())
    }

    /// Create a new tmux session
    pub fn new_session(name: &str, path: &std::path::Path, start_claude: bool) -> Result<()> {
        let path_str = path.to_string_lossy();

        let status = Command::new("tmux")
            .args(["new-session", "-d", "-s", name, "-c", &path_str])
            .status()
            .context("Failed to create new session")?;

        if !status.success() {
            anyhow::bail!("Failed to create session {}", name);
        }

        if start_claude {
            // Send claude command to the new session
            let _ = Command::new("tmux")
                .args(["send-keys", "-t", name, "claude", "Enter"])
                .status();
        }

        Ok(())
    }

    /// Kill a tmux session
    pub fn kill_session(session: &str) -> Result<()> {
        let status = Command::new("tmux")
            .args(["kill-session", "-t", session])
            .status()
            .context("Failed to kill session")?;

        if !status.success() {
            anyhow::bail!("Failed to kill session {}", session);
        }

        Ok(())
    }

    /// Rename a tmux session
    pub fn rename_session(old_name: &str, new_name: &str) -> Result<()> {
        let status = Command::new("tmux")
            .args(["rename-session", "-t", old_name, new_name])
            .status()
            .context("Failed to rename session")?;

        if !status.success() {
            anyhow::bail!("Failed to rename session {} to {}", old_name, new_name);
        }

        Ok(())
    }

    /// Get the current pane target (session:window.pane format)
    pub fn current_pane() -> Result<Option<String>> {
        let output = Command::new("tmux")
            .args([
                "display-message",
                "-p",
                "#{session_name}:#{window_index}.#{pane_index}",
            ])
            .output()
            .context("Failed to get current pane")?;

        if !output.status.success() {
            return Ok(None);
        }

        let target = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if target.is_empty() {
            Ok(None)
        } else {
            Ok(Some(target))
        }
    }
}
