use std::path::{Path, PathBuf};

/// Index into the flat `repos: Vec<Repo>`.
pub type RepoId = usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoStatus {
    Idle,
    Running,
    Done(i32),
    Failed,
    /// Killed by cancel after it had started running.
    Cancelled,
    /// Not yet started when cancel was requested (never ran).
    Skipped,
}

#[derive(Debug, Clone)]
pub struct Repo {
    pub fqdn: String,
    pub user: String,
    pub name: String,
    pub path: PathBuf,
    pub checked: bool,
    pub status: RepoStatus,
}

impl Repo {
    /// `user/repo` label used in output box headers etc.
    pub fn slug(&self) -> String {
        format!("{}/{}", self.user, self.name)
    }

    /// `fqdn/user/repo` relative path used for glob matching.
    pub fn rel_path(&self) -> String {
        format!("{}/{}/{}", self.fqdn, self.user, self.name)
    }
}

/// Default scan root (`~/src`). Falls back to `./src` if home can't be resolved.
pub fn default_root() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("src")
}

/// Collect dirs where `<root>/<fqdn>/<user>/<repo>/.git` exists as repos.
/// `.git` may be a dir or a file (file for worktrees / submodules). Fixed depth 3.
pub fn discover(root: &Path) -> Vec<Repo> {
    let mut repos = Vec::new();
    for fqdn_dir in subdirs(root) {
        let fqdn = dir_name(&fqdn_dir);
        for user_dir in subdirs(&fqdn_dir) {
            let user = dir_name(&user_dir);
            for repo_dir in subdirs(&user_dir) {
                if repo_dir.join(".git").exists() {
                    repos.push(Repo {
                        fqdn: fqdn.clone(),
                        user: user.clone(),
                        name: dir_name(&repo_dir),
                        path: repo_dir,
                        checked: false,
                        status: RepoStatus::Idle,
                    });
                }
            }
        }
    }
    repos.sort_by(|a, b| (&a.fqdn, &a.user, &a.name).cmp(&(&b.fqdn, &b.user, &b.name)));
    repos
}

/// Immediate subdirectories of `dir`, sorted by name (symlinks are followed).
fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

fn dir_name(path: &Path) -> String {
    path.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string()
}
