# Git & Worktree Awareness Design Document

This document outlines the staged implementation plan for adding git and worktree awareness to claude-tmux.

## Overview

**Goal**: Make sessions git-aware by detecting git metadata from session working directories, enabling contextual actions like showing branch names, dirty state indicators, and offering worktree deletion when killing sessions.

**Key Design Decisions**:
- Use `git2` crate (libgit2 bindings) instead of shelling out to git
- Git context is *enrichment* on existing sessions, not a separate concept
- h/l keys repurposed for sub-menu navigation (actions menu)
- Staged rollout to maintain stability

---

## Stage 1: Git Metadata Detection & Display

**Goal**: Detect basic git info and display it in the session list.

### Data Model

```rust
// src/git.rs (new file)

/// Git context for a session's working directory
pub struct GitContext {
    /// Current branch name (or HEAD if detached)
    pub branch: String,
    /// Whether the working directory has uncommitted changes
    pub is_dirty: bool,
    /// Whether this directory is a worktree (not the main checkout)
    pub is_worktree: bool,
    /// Path to the main repository (if this is a worktree)
    pub main_repo_path: Option<PathBuf>,
}

impl GitContext {
    /// Detect git context for a given path. Returns None if not a git repo.
    pub fn detect(path: &Path) -> Option<Self> { ... }
}
```

```rust
// src/session.rs - extend Session

pub struct Session {
    // ... existing fields ...

    /// Git context, if the working directory is a git repository
    pub git_context: Option<GitContext>,
}
```

### Implementation Details

**Detection using git2**:
```rust
use git2::Repository;

pub fn detect(path: &Path) -> Option<GitContext> {
    let repo = Repository::discover(path).ok()?;

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

    // Check dirty state
    let is_dirty = repo.statuses(None)
        .map(|statuses| !statuses.is_empty())
        .unwrap_or(false);

    // Check if worktree
    let is_worktree = repo.is_worktree();
    let main_repo_path = if is_worktree {
        repo.commondir().map(|p| p.to_path_buf())
    } else {
        None
    };

    Some(GitContext {
        branch,
        is_dirty,
        is_worktree,
        main_repo_path,
    })
}
```

### UI Changes

**Session list format**:
```
● my-feature        Working    ~/repos/project (feature-123) *
○ main-dev          Idle       ~/repos/project (main)
◐ review-pr         Waiting    ~/repos/other [pr-456]        # [] for worktrees
? legacy            Unknown    ~/old-stuff                    # no git info
```

**Legend**:
- `(branch)` - regular git repo with branch name
- `[branch]` - git worktree with branch name
- `*` suffix - dirty working directory
- No annotation - not a git repo

**Changes to `ui.rs`**:
- Modify `render_session_list()` to append git info after path
- Add styling: branch in cyan, `*` in yellow, `[]` brackets in magenta

### Files to Modify/Create

| File | Changes |
|------|---------|
| `Cargo.toml` | Add `git2` dependency |
| `src/git.rs` | **New file** - GitContext struct and detection |
| `src/session.rs` | Add `git_context: Option<GitContext>` field |
| `src/tmux.rs` | Call `GitContext::detect()` when building sessions |
| `src/ui.rs` | Render git info in session list |
| `src/lib.rs` or `main.rs` | Add `mod git;` |

### Performance Considerations

- `git2` is fast (no subprocess overhead)
- `Repository::discover()` walks up to find `.git` - usually instant
- `repo.statuses()` can be slow on large repos with many files
  - Consider: only check dirty state for selected session?
  - Or: use `StatusOptions` to limit scope

---

## Stage 2: Sub-Menu Navigation System

**Goal**: Repurpose h/l for navigating into action sub-menus instead of expand/collapse.

### New Navigation Model

```
Normal Mode (session list)
    │
    ├── j/k/↑/↓    Navigate sessions
    ├── l/→        Enter action menu for selected session
    ├── Enter      Go to session
    └── q/Esc      Quit

Action Menu Mode (for a specific session)
    │
    ├── j/k/↑/↓   Navigate actions
    ├── Enter  Execute selected action
    ├── h/←/Esc    Back to session list
    └── q          Quit entirely
```

### Data Model Changes

```rust
// src/app.rs

pub enum Mode {
    Normal,
    ActionMenu,      // NEW: viewing actions for selected session
    Filter,
    ConfirmAction,   // RENAMED from ConfirmKill - now generic
    NewSession,
    Rename,
    Help,
}

pub struct App {
    // ... existing fields ...

    /// Available actions for the selected session (computed)
    pub available_actions: Vec<SessionAction>,
    /// Currently highlighted action in ActionMenu mode
    pub selected_action: usize,
    /// Action pending confirmation
    pub pending_action: Option<SessionAction>,
}

#[derive(Clone)]
pub enum SessionAction {
    SwitchTo,
    Kill { delete_worktree: bool },
    Rename,
    // Future actions can be added here
}

impl SessionAction {
    pub fn label(&self) -> &str {
        match self {
            Self::SwitchTo => "Switch to session",
            Self::Kill { delete_worktree: false } => "Kill session",
            Self::Kill { delete_worktree: true } => "Kill session + delete worktree",
            Self::Rename => "Rename session",
        }
    }

    pub fn requires_confirmation(&self) -> bool {
        matches!(self, Self::Kill { .. })
    }
}
```

### Action Menu Logic

```rust
impl App {
    /// Compute available actions for the selected session
    pub fn compute_actions(&mut self) {
        let Some(session) = self.selected_session() else {
            self.available_actions = vec![];
            return;
        };

        let mut actions = vec![
            SessionAction::SwitchTo,
            SessionAction::Rename,
        ];

        // Kill action - with worktree option if applicable
        if let Some(git) = &session.git_context {
            if git.is_worktree {
                actions.push(SessionAction::Kill { delete_worktree: false });
                actions.push(SessionAction::Kill { delete_worktree: true });
            } else {
                actions.push(SessionAction::Kill { delete_worktree: false });
            }
        } else {
            actions.push(SessionAction::Kill { delete_worktree: false });
        }

        self.available_actions = actions;
        self.selected_action = 0;
    }

    pub fn enter_action_menu(&mut self) {
        self.compute_actions();
        self.mode = Mode::ActionMenu;
    }

    pub fn execute_selected_action(&mut self) {
        let action = self.available_actions[self.selected_action].clone();
        if action.requires_confirmation() {
            self.pending_action = Some(action);
            self.mode = Mode::ConfirmAction;
        } else {
            self.perform_action(action);
        }
    }
}
```

### UI Changes

**Action menu rendering** (right side panel or overlay):
```
┌─ Actions: my-feature ─────────────┐
│                                   │
│  > Switch to session              │
│    Rename session                 │
│    Stage changes to git           │ (if dirty)
│    Commit changes                 │ (if changes staged)
│    Push to remote                 │ (if ahead of remote)
│    Pull from remote               │ (if (behind and) clean)
│    Kill session                   │
│    Kill session + delete worktree │
│                                   │
│  [Enter] Select  [h/Esc] Back     │
└───────────────────────────────────┘
```

### Input Changes

```rust
// src/input.rs

fn handle_normal_mode(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        // Navigation
        KeyCode::Char('j') | KeyCode::Down => app.select_next(),
        KeyCode::Char('k') | KeyCode::Up => app.select_prev(),

        // Enter action menu (CHANGED from expand/collapse)
        KeyCode::Char('l') | KeyCode::Right | KeyCode::Enter => {
            app.enter_action_menu();
        }

        // ... rest unchanged ...
    }
}

fn handle_action_menu_mode(app: &mut App, key: KeyEvent) -> bool {
    match key.code {
        // Navigate actions
        KeyCode::Char('j') | KeyCode::Down => {
            app.selected_action = (app.selected_action + 1) % app.available_actions.len();
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.selected_action = app.selected_action.saturating_sub(1);
        }

        // Execute action
        KeyCode::Char('l') | KeyCode::Right | KeyCode::Enter => {
            app.execute_selected_action();
        }

        // Back to session list
        KeyCode::Char('h') | KeyCode::Left | KeyCode::Esc => {
            app.mode = Mode::Normal;
        }

        // Quit entirely
        KeyCode::Char('q') => return true,

        _ => {}
    }
    false
}
```

### Files to Modify

| File | Changes |
|------|---------|
| `src/app.rs` | Add `SessionAction` enum, action menu state, methods |
| `src/input.rs` | Add `handle_action_menu_mode()`, change h/l behavior |
| `src/ui.rs` | Add `render_action_menu()` |

### Migration Notes

- **Breaking change**: h/l no longer expand/collapse details
- Details could be shown automatically in action menu, or via a dedicated key (e.g., `d` for details)
- Consider: show session details in action menu header

---

## Stage 3: Worktree Actions

**Goal**: Implement actual worktree deletion when "Kill session + delete worktree" is selected.

### Git Module Extensions

```rust
// src/git.rs additions

impl GitContext {
    /// Delete the worktree at the given path
    /// This removes the worktree from git and optionally the directory
    pub fn delete_worktree(worktree_path: &Path, force: bool) -> Result<()> {
        let repo = Repository::discover(worktree_path)?;

        // Find the worktree by path
        let worktrees = repo.worktrees()?;
        for name in worktrees.iter() {
            let name = name.ok_or_else(|| anyhow!("Invalid worktree name"))?;
            let wt = repo.find_worktree(name)?;
            if wt.path() == worktree_path {
                // Validate it's safe to delete
                if !force {
                    wt.validate()?; // Ensures no uncommitted changes, etc.
                }
                wt.prune(PruneOptions::new())?;

                // Remove the directory
                std::fs::remove_dir_all(worktree_path)?;
                return Ok(());
            }
        }

        Err(anyhow!("Worktree not found"))
    }
}
```

### Safety Checks

Before deleting a worktree, verify:
1. It actually is a worktree (not main repo)
2. Working directory is clean (or user confirms force)
3. No other tmux sessions are using this path

```rust
impl App {
    fn can_delete_worktree(&self, session: &Session) -> Result<(), &'static str> {
        let git = session.git_context.as_ref()
            .ok_or("Not a git repository")?;

        if !git.is_worktree {
            return Err("Not a worktree");
        }

        if git.is_dirty {
            return Err("Worktree has uncommitted changes");
        }

        // Check if other sessions use this path
        let path = &session.working_directory;
        let other_sessions = self.sessions.iter()
            .filter(|s| s.name != session.name && s.working_directory == *path)
            .count();

        if other_sessions > 0 {
            return Err("Other sessions are using this worktree");
        }

        Ok(())
    }
}
```

### Confirmation Dialog Enhancement

```
┌─ Confirm Action ──────────────────────────────┐
│                                               │
│  Kill session "my-feature"                    │
│  AND delete worktree at ~/repos/project-wt    │
│                                               │
│  ⚠ This will permanently delete the           │
│    worktree directory                         │
│                                               │
│  [y] Confirm    [n] Cancel                    │
└───────────────────────────────────────────────┘
```

### Files to Modify

| File | Changes |
|------|---------|
| `src/git.rs` | Add `delete_worktree()` function |
| `src/app.rs` | Add `can_delete_worktree()`, implement worktree deletion in `perform_action()` |
| `src/ui.rs` | Enhance confirmation dialog for worktree actions |

---

## Stage 4: Polish & Edge Cases

**Goal**: Handle edge cases, improve UX, add tests.

### Edge Cases to Handle

1. **Detached HEAD**: Show short commit hash instead of branch name
2. **Empty repository**: Handle repos with no commits
3. **Bare repository**: Skip git detection for bare repos
4. **Nested worktrees**: Unlikely but possible
5. **Permission errors**: Handle gracefully if `.git` isn't readable
6. **Submodules**: Should show submodule's branch, not parent's

### Testing

```rust
// src/git.rs tests

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use git2::Repository;

    #[test]
    fn test_detect_regular_repo() {
        let dir = TempDir::new().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        // Create initial commit...

        let ctx = GitContext::detect(dir.path()).unwrap();
        assert_eq!(ctx.branch, "main"); // or "master" depending on git config
        assert!(!ctx.is_worktree);
        assert!(!ctx.is_dirty);
    }

    #[test]
    fn test_detect_worktree() {
        // Create main repo
        // Create worktree
        // Verify detection
    }

    #[test]
    fn test_detect_dirty_state() {
        // Create repo, modify file, check is_dirty
    }

    #[test]
    fn test_non_git_directory() {
        let dir = TempDir::new().unwrap();
        assert!(GitContext::detect(dir.path()).is_none());
    }
}
```

### Documentation Updates

- Update README with new keybindings (h/l for action menu)
- Document git-aware features
- Add screenshots showing git info in session list

---

## Implementation Order

| Stage | Deliverable | Depends On |
|-------|-------------|------------|
| 1a | `git.rs` with `GitContext::detect()` | - |
| 1b | Session struct extended with git_context | 1a |
| 1c | UI shows git info (branch, dirty, worktree indicator) | 1b |
| 2a | `SessionAction` enum and action menu state | - |
| 2b | Action menu UI rendering | 2a |
| 2c | h/l navigation changes | 2a, 2b |
| 3a | `delete_worktree()` implementation | 1a |
| 3b | Safety checks before deletion | 1b, 3a |
| 3c | Enhanced confirmation dialog | 2b, 3b |
| 4 | Edge cases, tests, docs | All above |

---

## Open Questions

1. **Dirty state performance**: Should we check dirty state eagerly for all sessions, or lazily only for the selected session? Eagerly.

2. **Details panel**: With h/l repurposed, how should users see expanded session details? No.

3. **Quick actions**: Should some actions (like switch-to) remain accessible directly from normal mode (Enter), or always go through action menu? Enter in normal mode switches immediately.

4. **Future actions**: What other actions might benefit from the action menu?
   - Open in editor
   - Copy path to clipboard
   - Create new session from same worktree
   - Git operations (fetch, pull)?

---

## Dependencies

```toml
# Cargo.toml additions
[dependencies]
git2 = "0.20"  # or latest stable
```
