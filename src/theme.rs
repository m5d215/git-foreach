use serde::Deserialize;

/// Icon representation: Nerd Font glyphs or plain ASCII.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IconMode {
    #[default]
    Nerd,
    Ascii,
}

/// The icon set used for rendering. Callers treat icons as fixed-width slots to
/// guard against cell-width drift.
pub struct Icons {
    pub expanded: &'static str,
    pub collapsed: &'static str,
    pub fqdn: &'static str,
    pub user: &'static str,
    pub repo: &'static str,
    pub status_done: &'static str,
    pub status_fail: &'static str,
    pub status_cancel: &'static str,
    pub status_skip: &'static str,
    /// Spinner frames for the running state.
    pub spinner: &'static [&'static str],
    pub focus: &'static str,
    pub box_bar: &'static str,
    /// Cancel button at the right end of the command line.
    pub cancel: &'static str,
    /// Copy button floating at the top-right of the output pane.
    pub copy: &'static str,
    // checkbox
    pub check_on: &'static str,
    pub check_off: &'static str,
    pub check_partial: &'static str,
    // selection accent bar / prompt
    pub sel_bar: &'static str,
    pub prompt: &'static str,
    // powerline pill (nerd only; ascii sets pills=false and falls back to a bar)
    pub pills: bool,
    pub pl_left: &'static str,
    pub pl_sep: &'static str,
    pub pl_right: &'static str,
}

// Nerd Font glyphs live in the PUA, so they are written as explicit codepoints
// (\u{...}). All are Font Awesome glyphs bundled in Nerd Fonts; swap by codepoint.
const NERD: Icons = Icons {
    expanded: "\u{f078}",      // chevron-down
    collapsed: "\u{f054}",     // chevron-right
    fqdn: "\u{f0ac}",          // globe
    user: "\u{f007}",          // user
    repo: "\u{f126}",          // code-branch
    status_done: "\u{f00c}",   // check
    status_fail: "\u{f00d}",   // times
    status_cancel: "\u{f05e}", // ban
    status_skip: "\u{f068}",   // minus
    spinner: &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
    focus: "\u{f06e}", // eye
    box_bar: "▌",
    cancel: "\u{f04d}",        // stop
    copy: "\u{f0c5}",          // copy (two pages)
    check_on: "\u{f046}",      // check-square
    check_off: "\u{f096}",     // square-o
    check_partial: "\u{f147}", // minus-square
    sel_bar: "▎",
    prompt: "\u{f061}", // arrow-right
    pills: true,
    pl_left: "\u{e0b6}",  // rounded left
    pl_sep: "\u{e0b0}",   // triangle right
    pl_right: "\u{e0b4}", // rounded right
};

const ASCII: Icons = Icons {
    expanded: "v",
    collapsed: ">",
    fqdn: "",
    user: "",
    repo: "",
    status_done: "ok",
    status_fail: "x",
    status_cancel: "!",
    status_skip: "-",
    spinner: &["|", "/", "-", "\\"],
    focus: "*",
    box_bar: "|",
    cancel: "stop",
    copy: "copy",
    check_on: "[x]",
    check_off: "[ ]",
    check_partial: "[~]",
    sel_bar: ">",
    prompt: ">",
    pills: false,
    pl_left: "",
    pl_sep: "",
    pl_right: "",
};

impl Icons {
    pub fn new(mode: IconMode) -> &'static Icons {
        match mode {
            IconMode::Nerd => &NERD,
            IconMode::Ascii => &ASCII,
        }
    }
}
