use crate::app::Pane;
use crate::repo::RepoId;
use crate::tree::NodeRef;

/// Single hub for every operation. Mouse, default keys, and the config keymap all
/// convert into an `Action`, and `App::apply` looks only at this.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)] // variants are wired up across phases 2..8
pub enum Action {
    // tree
    ToggleCheck(NodeRef),
    Focus(RepoId),
    ClearFocus,
    Expand(NodeRef),
    Collapse(NodeRef),
    CursorUp,
    CursorDown,
    CheckAll,
    UncheckAll,
    // command
    LoadPreset(usize),
    Run,
    Cancel,
    // output
    ScrollUp,
    ScrollDown,
    ScrollTop,
    ScrollBottom,
    // app
    Rescan,
    FocusPane(Pane),
    Quit,
}
