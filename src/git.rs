use std::path::{Path, PathBuf};

use git2::{Repository, StatusOptions};

/// Git context for a session's working directory
#[derive(Debug, Clone)]
pub struct GitContext {
    /// Current branch name (or short commit hash if detached)
    pub branch: String,
    /// Whether the working directory has uncommitted changes
    pub is_dirty: bool,
    /// Whether this directory is a worktree (not the main checkout)
    pub is_worktree: bool,
    /// Path to the main repository (if this is a worktree)
    pub main_repo_path: Option<PathBuf>,
    /// Commits ahead of upstream
    pub ahead: usize,
    /// Commits behind upstream
    pub behind: usize,
}

impl GitContext {
    /// Detect git context for a given path. Returns None if not a git repo.
    pub fn detect(path: &Path) -> Option<Self> {
        let repo = Repository::discover(path).ok()?;

        // Skip bare repositories
        if repo.is_bare() {
            return None;
        }

        // Get branch name
        let branch = match repo.head() {
            Ok(head) => {
                if head.is_branch() {
                    head.shorthand().unwrap_or("HEAD").to_string()
                } else {
                    // Detached HEAD - show short commit hash
                    head.peel_to_commit()
                        .map(|c| c.id().to_string()[..7].to_string())
                        .unwrap_or_else(|_| "HEAD".to_string())
                }
            }
            Err(_) => "HEAD".to_string(), // Empty repo or other edge case
        };

        // Check dirty state with explicit options to match `git status` behavior
        let mut status_opts = StatusOptions::new();
        status_opts
            .include_untracked(true)
            .include_ignored(false)
            .exclude_submodules(true);

        let is_dirty = repo
            .statuses(Some(&mut status_opts))
            .map(|statuses| !statuses.is_empty())
            .unwrap_or(false);

        // Check if worktree
        let is_worktree = repo.is_worktree();
        let main_repo_path = if is_worktree {
            Some(repo.commondir().to_path_buf())
        } else {
            None
        };

        // Check ahead/behind upstream
        let (ahead, behind) = Self::get_ahead_behind(&repo).unwrap_or((0, 0));

        Some(GitContext {
            branch,
            is_dirty,
            is_worktree,
            main_repo_path,
            ahead,
            behind,
        })
    }

    /// Get the number of commits ahead/behind the upstream branch
    fn get_ahead_behind(repo: &Repository) -> Option<(usize, usize)> {
        let head = repo.head().ok()?;
        if !head.is_branch() {
            return None; // Detached HEAD has no upstream
        }

        let branch_name = head.shorthand()?;
        let local_branch = repo.find_branch(branch_name, git2::BranchType::Local).ok()?;
        let upstream = local_branch.upstream().ok()?;

        let local_oid = head.target()?;
        let upstream_oid = upstream.get().target()?;

        repo.graph_ahead_behind(local_oid, upstream_oid).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_non_git_directory() {
        let dir = std::env::temp_dir();
        // temp_dir itself is unlikely to be a git repo
        // but we can't guarantee it, so just test the function doesn't panic
        let _ = GitContext::detect(&dir);
    }
}
