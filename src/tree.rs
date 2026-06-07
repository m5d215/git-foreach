use crate::repo::{Repo, RepoId};

/// Positional reference to a tree node. Mouse builds it from the click position,
/// keyboard from the cursor row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeRef {
    Fqdn(usize),
    User(usize, usize),
    Repo(RepoId),
}

pub struct UserNode {
    pub name: String,
    pub expanded: bool,
    pub repos: Vec<RepoId>,
}

pub struct FqdnNode {
    pub name: String,
    pub expanded: bool,
    pub users: Vec<UserNode>,
}

/// One flattened row for rendering.
pub struct VisibleRow {
    pub node: NodeRef,
    pub indent: u16,
}

/// Display model of the tree: hierarchy, expansion, and cursor. A repo's
/// checked/status lives in `App.repos` (the flat Vec); here we only reference RepoId.
pub struct TreeView {
    pub fqdns: Vec<FqdnNode>,
    pub cursor: usize,
}

impl TreeView {
    /// Build by grouping the sorted `repos` as fqdn → user. Starts fully expanded.
    pub fn build(repos: &[Repo]) -> Self {
        let mut fqdns: Vec<FqdnNode> = Vec::new();
        for (id, repo) in repos.iter().enumerate() {
            let fi = match fqdns.iter().position(|f| f.name == repo.fqdn) {
                Some(i) => i,
                None => {
                    fqdns.push(FqdnNode {
                        name: repo.fqdn.clone(),
                        expanded: true,
                        users: Vec::new(),
                    });
                    fqdns.len() - 1
                }
            };
            let users = &mut fqdns[fi].users;
            let ui = match users.iter().position(|u| u.name == repo.user) {
                Some(i) => i,
                None => {
                    users.push(UserNode {
                        name: repo.user.clone(),
                        expanded: true,
                        repos: Vec::new(),
                    });
                    users.len() - 1
                }
            };
            users[ui].repos.push(id);
        }
        Self { fqdns, cursor: 0 }
    }

    /// Visible rows reflecting the current expansion state.
    pub fn visible(&self) -> Vec<VisibleRow> {
        let mut rows = Vec::new();
        for (fi, f) in self.fqdns.iter().enumerate() {
            rows.push(VisibleRow {
                node: NodeRef::Fqdn(fi),
                indent: 0,
            });
            if f.expanded {
                for (ui, u) in f.users.iter().enumerate() {
                    rows.push(VisibleRow {
                        node: NodeRef::User(fi, ui),
                        indent: 1,
                    });
                    if u.expanded {
                        for &rid in &u.repos {
                            rows.push(VisibleRow {
                                node: NodeRef::Repo(rid),
                                indent: 2,
                            });
                        }
                    }
                }
            }
        }
        rows
    }

    pub fn cursor_node(&self) -> Option<NodeRef> {
        self.visible().get(self.cursor).map(|r| r.node)
    }

    pub fn move_cursor(&mut self, delta: isize) {
        let len = self.visible().len();
        if len == 0 {
            return;
        }
        let next = (self.cursor as isize + delta).clamp(0, len as isize - 1);
        self.cursor = next as usize;
    }

    pub fn is_expanded(&self, node: NodeRef) -> bool {
        match node {
            NodeRef::Fqdn(i) => self.fqdns.get(i).map(|f| f.expanded).unwrap_or(false),
            NodeRef::User(i, j) => self
                .fqdns
                .get(i)
                .and_then(|f| f.users.get(j))
                .map(|u| u.expanded)
                .unwrap_or(false),
            NodeRef::Repo(_) => false,
        }
    }

    pub fn set_expanded(&mut self, node: NodeRef, expanded: bool) {
        match node {
            NodeRef::Fqdn(i) => {
                if let Some(f) = self.fqdns.get_mut(i) {
                    f.expanded = expanded;
                }
            }
            NodeRef::User(i, j) => {
                if let Some(u) = self.fqdns.get_mut(i).and_then(|f| f.users.get_mut(j)) {
                    u.expanded = expanded;
                }
            }
            NodeRef::Repo(_) => {}
        }
    }

    /// All RepoIds under a node (for bulk toggle / tri-state).
    pub fn repo_ids(&self, node: NodeRef) -> Vec<RepoId> {
        match node {
            NodeRef::Repo(id) => vec![id],
            NodeRef::User(i, j) => self
                .fqdns
                .get(i)
                .and_then(|f| f.users.get(j))
                .map(|u| u.repos.clone())
                .unwrap_or_default(),
            NodeRef::Fqdn(i) => self
                .fqdns
                .get(i)
                .map(|f| f.users.iter().flat_map(|u| u.repos.iter().copied()).collect())
                .unwrap_or_default(),
        }
    }

    /// Label of a group node (repo labels come from `App.repos`).
    pub fn group_label(&self, node: NodeRef) -> &str {
        match node {
            NodeRef::Fqdn(i) => self.fqdns.get(i).map(|f| f.name.as_str()).unwrap_or(""),
            NodeRef::User(i, j) => self
                .fqdns
                .get(i)
                .and_then(|f| f.users.get(j))
                .map(|u| u.name.as_str())
                .unwrap_or(""),
            NodeRef::Repo(_) => "",
        }
    }
}
