use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarOrientation,
    ScrollbarState,
};

use crate::action::Action;
use crate::config::{Config, Preset};
use crate::output::{LineKind, RepoOutput};
use crate::repo::{self, Repo, RepoId, RepoStatus};
use crate::runner::{Runner, RunnerEvent, Target};
use crate::theme::Icons;
use crate::tree::{NodeRef, TreeView};
use tui_input::backend::crossterm::EventHandler;
use tui_input::{Input, InputRequest};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Tree,
    Output,
    Input,
}

/// Aggregate check state of a group node (for tri-state display).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckState {
    None,
    Partial,
    All,
}

/// A hit-test target. Recorded with its Rect during render and looked up by click coords.
#[derive(Debug, Clone, Copy)]
enum Hit {
    /// A region that simply fires this Action (e.g. a button).
    Fire(Action),
    /// A tree row; the column decides checkbox / expand marker / name.
    TreeRow {
        node: NodeRef,
        row_index: usize,
        /// Content start x for the row (inside any border).
        text_x: u16,
        indent: u16,
    },
}

/// Whole application state.
pub struct App {
    pub should_quit: bool,
    pub root: PathBuf,
    pub repos: Vec<Repo>,
    pub tree: TreeView,
    pub active_pane: Pane,
    /// Output-pane focus (None = show all repos). Independent of checkbox.
    pub focus: Option<RepoId>,
    pub input: Input,
    /// Submitted commands, oldest first. Browsed with ↑/↓ while editing.
    history: Vec<String>,
    /// Position in `history` while browsing (None = editing the live draft).
    history_pos: Option<usize>,
    /// The live input stashed when history browsing starts, restored on the way back.
    history_draft: String,
    pub outputs: HashMap<RepoId, RepoOutput>,
    pub running: bool,
    concurrency: usize,
    /// Number of in-flight repos; running clears at 0.
    pending: usize,
    rx: Option<Receiver<RunnerEvent>>,
    runner: Runner,
    tree_state: ListState,
    output_scroll: u16,
    /// When true, follow the bottom on new output; manual scroll sets false.
    follow: bool,
    /// Hit-test table, rebuilt every render.
    hit: Vec<(Rect, Hit)>,
    tree_area: Rect,
    output_area: Rect,
    input_area: Rect,
    icons: &'static Icons,
    presets: Vec<Preset>,
    /// Key string → Action name (config keymap merged with preset.key).
    keymap: HashMap<String, String>,
    /// Status line for startup config errors etc.
    notice: Option<String>,
    /// Render counter for the spinner animation.
    tick: usize,
    /// When the copy button was last fired and whether it succeeded; drives a
    /// brief green/red color flash.
    copy_flash: Option<(Instant, bool)>,
}

impl App {
    pub fn new() -> Self {
        let (config, notice) = Config::load();
        Self::from_config(config, notice)
    }

    /// Build from an explicit config. `new()` loads the user config; tests pass
    /// `Config::default()` so they don't depend on the machine's config file.
    fn from_config(config: Config, notice: Option<String>) -> Self {
        let root = repo::default_root();
        let mut repos = repo::discover(&root);
        apply_default_checked(&mut repos, &config.default_checked);

        let tree = TreeView::build(&repos);
        let mut tree_state = ListState::default();
        tree_state.select(Some(0));
        let concurrency = std::thread::available_parallelism()
            .map(|n| n.get().min(8))
            .unwrap_or(8);

        let icons = Icons::new(config.icons);
        let keymap = build_keymap(&config);

        Self {
            should_quit: false,
            root,
            repos,
            tree,
            active_pane: Pane::Tree,
            focus: None,
            input: Input::default(),
            history: Vec::new(),
            history_pos: None,
            history_draft: String::new(),
            outputs: HashMap::new(),
            running: false,
            concurrency,
            pending: 0,
            rx: None,
            runner: Runner::new(),
            tree_state,
            output_scroll: 0,
            follow: true,
            hit: Vec::new(),
            tree_area: Rect::default(),
            output_area: Rect::default(),
            input_area: Rect::default(),
            icons,
            presets: config.presets,
            keymap,
            notice,
            tick: 0,
            copy_flash: None,
        }
    }

    // --- input---------------------------------------------------------------

    pub fn on_key(&mut self, key: KeyEvent) {
        // Always-on safety valve.
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.apply(Action::Quit);
            return;
        }
        if key.code == KeyCode::Tab {
            self.cycle_pane();
            return;
        }
        if self.active_pane == Pane::Input {
            self.handle_input_key(key);
            return;
        }
        // Config keymap (extra bindings) takes precedence over built-ins.
        if let Some(action) = self.keymap_action(key) {
            self.apply(action);
            return;
        }
        if let Some(action) = self.map_nav_key(key) {
            self.apply(action);
        }
    }

    fn keymap_action(&self, key: KeyEvent) -> Option<Action> {
        let s = key_to_string(key)?;
        let name = self.keymap.get(&s)?;
        self.resolve_named_action(name)
    }

    /// Config keymap Action name → Action. Cursor-relative ones act on the current cursor.
    fn resolve_named_action(&self, name: &str) -> Option<Action> {
        let cursor = self.tree.cursor_node();
        match name {
            "rescan" => Some(Action::Rescan),
            "cancel" => Some(Action::Cancel),
            "run" => Some(Action::Run),
            "quit" => Some(Action::Quit),
            "cursor_up" => Some(Action::CursorUp),
            "cursor_down" => Some(Action::CursorDown),
            "check_all" => Some(Action::CheckAll),
            "uncheck_all" => Some(Action::UncheckAll),
            "clear_focus" => Some(Action::ClearFocus),
            "scroll_up" => Some(Action::ScrollUp),
            "scroll_down" => Some(Action::ScrollDown),
            "scroll_top" => Some(Action::ScrollTop),
            "scroll_bottom" => Some(Action::ScrollBottom),
            "copy_output" => Some(Action::CopyOutput),
            "input" => Some(Action::FocusPane(Pane::Input)),
            "toggle_check" => cursor.map(Action::ToggleCheck),
            "expand" => cursor.map(Action::Expand),
            "collapse" => cursor.map(Action::Collapse),
            "toggle_expand" => cursor.map(|n| toggle_expand(&self.tree, n)),
            "focus" => match cursor {
                Some(NodeRef::Repo(id)) => Some(Action::Focus(id)),
                _ => None,
            },
            other => other
                .strip_prefix("preset:")
                .and_then(|n| n.parse::<usize>().ok())
                .map(Action::LoadPreset),
        }
    }

    /// Text editing while the input pane is active. Enter / Esc / history keys are
    /// handled here; everything else (cursor moves, word ops, kill/yank) is delegated
    /// to tui-input's readline-style handler.
    fn handle_input_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter => self.apply(Action::Run),
            KeyCode::Esc => self.active_pane = Pane::Tree,
            KeyCode::Up => self.history_prev(),
            KeyCode::Down => self.history_next(),
            _ => {
                self.input.handle_event(&Event::Key(key));
            }
        }
    }

    /// Insert pasted text (bracketed paste) at the cursor. Control characters are
    /// dropped so a multi-line paste collapses into a single command line.
    pub fn on_paste(&mut self, data: String) {
        if self.active_pane != Pane::Input {
            return;
        }
        for c in data.chars().filter(|c| !c.is_control()) {
            self.input.handle(InputRequest::InsertChar(c));
        }
    }

    /// Step back to an older history entry (↑).
    fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let pos = match self.history_pos {
            None => {
                self.history_draft = self.input.value().to_string();
                self.history.len() - 1
            }
            Some(0) => 0,
            Some(i) => i - 1,
        };
        self.history_pos = Some(pos);
        self.input = Input::new(self.history[pos].clone());
    }

    /// Step forward toward newer entries (↓); past the newest restores the draft.
    fn history_next(&mut self) {
        let Some(i) = self.history_pos else {
            return;
        };
        if i + 1 < self.history.len() {
            self.history_pos = Some(i + 1);
            self.input = Input::new(self.history[i + 1].clone());
        } else {
            self.history_pos = None;
            self.input = Input::new(std::mem::take(&mut self.history_draft));
        }
    }

    /// Default keys → Action in nav (non-input) mode. The final set is minimal, but
    /// for now a full set is bound so it works without a mouse (trimmed in phase 7).
    fn map_nav_key(&self, key: KeyEvent) -> Option<Action> {
        let cursor = self.tree.cursor_node();
        match (key.modifiers, key.code) {
            (_, KeyCode::Char('q')) => Some(Action::Quit),
            (_, KeyCode::Char('i') | KeyCode::Char('/')) => Some(Action::FocusPane(Pane::Input)),
            (_, KeyCode::Down | KeyCode::Char('j')) => Some(Action::CursorDown),
            (_, KeyCode::Up | KeyCode::Char('k')) => Some(Action::CursorUp),
            (_, KeyCode::Right | KeyCode::Char('l')) => cursor.map(Action::Expand),
            (_, KeyCode::Left | KeyCode::Char('h')) => cursor.map(Action::Collapse),
            (_, KeyCode::Char(' ')) => cursor.map(Action::ToggleCheck),
            (_, KeyCode::Enter) => match cursor {
                Some(NodeRef::Repo(id)) => Some(Action::Focus(id)),
                _ => None,
            },
            (_, KeyCode::Char('a')) => Some(Action::CheckAll),
            (KeyModifiers::SHIFT, KeyCode::Char('A')) => Some(Action::UncheckAll),
            (_, KeyCode::Char('c')) => Some(Action::Cancel),
            (_, KeyCode::Char('r')) => Some(Action::Rescan),
            (_, KeyCode::Esc) => Some(Action::ClearFocus),
            (_, KeyCode::PageDown) => Some(Action::ScrollDown),
            (_, KeyCode::PageUp) => Some(Action::ScrollUp),
            (_, KeyCode::Home) => Some(Action::ScrollTop),
            (_, KeyCode::End) => Some(Action::ScrollBottom),
            _ => None,
        }
    }

    fn cycle_pane(&mut self) {
        self.active_pane = match self.active_pane {
            Pane::Tree => Pane::Output,
            Pane::Output => Pane::Input,
            Pane::Input => Pane::Tree,
        };
    }

    // --- mouse-------------------------------------------------------------

    pub fn on_mouse(&mut self, ev: MouseEvent) {
        match ev.kind {
            MouseEventKind::Down(MouseButton::Left) => self.handle_click(ev.column, ev.row),
            MouseEventKind::ScrollDown => self.handle_scroll(ev.column, ev.row, 3),
            MouseEventKind::ScrollUp => self.handle_scroll(ev.column, ev.row, -3),
            _ => {}
        }
    }

    fn handle_click(&mut self, col: u16, row: u16) {
        // Find the hit last-wins (topmost drawn element first).
        let hit = self
            .hit
            .iter()
            .rev()
            .find(|(rect, _)| contains(*rect, col, row))
            .map(|(_, h)| *h);

        match hit {
            Some(Hit::Fire(action)) => self.apply(action),
            Some(Hit::TreeRow {
                node,
                row_index,
                text_x,
                indent,
            }) => {
                self.active_pane = Pane::Tree;
                self.tree.cursor = row_index;
                self.tree_state.select(Some(row_index));
                if let Some(action) = tree_click_action(&self.tree, node, indent, text_x, col) {
                    self.apply(action);
                }
            }
            None => {
                if contains(self.input_area, col, row) {
                    self.active_pane = Pane::Input;
                } else if contains(self.output_area, col, row) {
                    self.active_pane = Pane::Output;
                } else if contains(self.tree_area, col, row) {
                    self.active_pane = Pane::Tree;
                }
            }
        }
    }

    fn handle_scroll(&mut self, col: u16, row: u16, delta: i32) {
        if contains(self.output_area, col, row) {
            if delta < 0 {
                self.apply(Action::ScrollUp);
            } else {
                self.apply(Action::ScrollDown);
            }
        } else if contains(self.tree_area, col, row) {
            self.tree.move_cursor(delta as isize / 3);
            self.tree_state.select(Some(self.tree.cursor));
        }
    }

    // --- apply actions--------------------------------------------------------

    pub fn apply(&mut self, action: Action) {
        match action {
            Action::Quit => {
                // Stop remaining children so they aren't orphaned, then exit.
                self.runner.cancel();
                self.should_quit = true;
            }
            Action::CursorDown => self.tree.move_cursor(1),
            Action::CursorUp => self.tree.move_cursor(-1),
            Action::Expand(node) => self.tree.set_expanded(node, true),
            Action::Collapse(node) => self.tree.set_expanded(node, false),
            Action::ToggleCheck(node) => self.toggle_check(node),
            Action::CheckAll => self.set_all_checked(true),
            Action::UncheckAll => self.set_all_checked(false),
            Action::Focus(id) => {
                self.focus = if self.focus == Some(id) {
                    None
                } else {
                    Some(id)
                };
                self.follow = true;
            }
            Action::ClearFocus => self.focus = None,
            Action::FocusPane(pane) => self.active_pane = pane,
            Action::Run => self.run_command(),
            Action::Cancel => self.runner.cancel(),
            Action::Rescan => self.rescan(),
            Action::ScrollUp => {
                self.output_scroll = self.output_scroll.saturating_sub(3);
                self.follow = false;
            }
            Action::ScrollDown => {
                self.output_scroll = self.output_scroll.saturating_add(3);
                self.follow = false;
            }
            Action::ScrollTop => {
                self.output_scroll = 0;
                self.follow = false;
            }
            Action::ScrollBottom => self.follow = true,
            Action::CopyOutput => self.copy_output(),
            Action::LoadPreset(i) => {
                if let Some(preset) = self.presets.get(i) {
                    self.input = Input::new(preset.command.clone());
                    self.history_pos = None;
                    self.active_pane = Pane::Input;
                }
            }
        }
        self.tree_state.select(Some(self.tree.cursor));
    }

    fn toggle_check(&mut self, node: NodeRef) {
        let ids = self.tree.repo_ids(node);
        let all_checked = !ids.is_empty() && ids.iter().all(|&id| self.repos[id].checked);
        for id in ids {
            self.repos[id].checked = !all_checked;
        }
    }

    fn set_all_checked(&mut self, checked: bool) {
        for repo in &mut self.repos {
            repo.checked = checked;
        }
    }

    fn check_state(&self, node: NodeRef) -> CheckState {
        let ids = self.tree.repo_ids(node);
        let checked = ids.iter().filter(|&&id| self.repos[id].checked).count();
        match checked {
            0 => CheckState::None,
            n if n == ids.len() => CheckState::All,
            _ => CheckState::Partial,
        }
    }

    fn rescan(&mut self) {
        if self.running {
            return;
        }
        self.repos = repo::discover(&self.root);
        self.tree = TreeView::build(&self.repos);
        self.tree_state.select(Some(0));
        self.outputs.clear();
        self.focus = None;
    }

    // --- execution---------------------------------------------------------------

    fn run_command(&mut self) {
        if self.running {
            return;
        }
        let command = self.input.value().trim().to_string();
        if command.is_empty() {
            return;
        }
        let targets: Vec<Target> = self
            .repos
            .iter()
            .enumerate()
            .filter(|(_, r)| r.checked)
            .map(|(id, r)| (id, r.path.clone()))
            .collect();
        if targets.is_empty() {
            return;
        }

        // Record in history (skip consecutive duplicates) and leave browsing mode.
        if self.history.last().map(String::as_str) != Some(command.as_str()) {
            self.history.push(command.clone());
        }
        self.history_pos = None;
        self.history_draft.clear();

        self.outputs.clear();
        for (id, _) in &targets {
            self.repos[*id].status = RepoStatus::Running;
            self.outputs.insert(*id, RepoOutput::default());
        }
        self.pending = targets.len();
        self.running = true;
        self.follow = true;
        self.output_scroll = 0;
        self.rx = Some(self.runner.start(command, targets, self.concurrency));
    }

    /// Called every main-loop iteration; drains worker events into state.
    pub fn drain_events(&mut self) {
        let events: Vec<RunnerEvent> = match &self.rx {
            Some(rx) => rx.try_iter().collect(),
            None => return,
        };
        for ev in events {
            self.handle_event(ev);
        }
    }

    fn handle_event(&mut self, ev: RunnerEvent) {
        match ev {
            RunnerEvent::Started { repo, command } => {
                self.outputs.entry(repo).or_default().push_command(&command);
            }
            RunnerEvent::Line { repo, stream, text } => {
                self.outputs
                    .entry(repo)
                    .or_default()
                    .push_stream(stream, text);
            }
            RunnerEvent::Finished { repo, code } => {
                if let Some(out) = self.outputs.get_mut(&repo) {
                    out.code = code;
                    out.finished = true;
                }
                // No code = killed by signal; in this tool the main cause is cancel kill.
                self.repos[repo].status = match code {
                    Some(c) => RepoStatus::Done(c),
                    None => RepoStatus::Cancelled,
                };
                self.mark_done();
            }
            RunnerEvent::Skipped { repo } => {
                if let Some(out) = self.outputs.get_mut(&repo) {
                    out.finished = true;
                }
                self.repos[repo].status = RepoStatus::Skipped;
                self.mark_done();
            }
            RunnerEvent::Error { repo, message } => {
                let out = self.outputs.entry(repo).or_default();
                out.push_stream(crate::runner::Stream::Stderr, format!("error: {message}"));
                out.finished = true;
                self.repos[repo].status = RepoStatus::Failed;
                self.mark_done();
            }
        }
    }

    fn mark_done(&mut self) {
        self.pending = self.pending.saturating_sub(1);
        if self.pending == 0 {
            self.running = false;
            self.rx = None;
        }
    }

    // --- rendering---------------------------------------------------------------

    pub fn render(&mut self, frame: &mut Frame) {
        self.hit.clear();
        self.tick = self.tick.wrapping_add(1);

        // Command region = top rule + (presets row) + input row. Presets sit below the rule, above input.
        let chips_h: u16 = if self.presets.is_empty() { 0 } else { 1 };
        let cmd_h = 2 + chips_h;
        let [body, cmd] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(cmd_h)]).areas(frame.area());

        let [tree_area, output_area] =
            Layout::horizontal([Constraint::Percentage(35), Constraint::Percentage(65)])
                .areas(body);
        self.tree_area = tree_area;
        self.output_area = output_area;

        self.render_tree(frame, tree_area);
        self.render_output(frame, output_area);

        // Top rule separating the body from the command region.
        let border = self.pane_border(Pane::Input);
        frame.render_widget(
            Block::default().borders(Borders::TOP).border_style(border),
            cmd,
        );
        let line = |y: u16| Rect {
            x: cmd.x,
            y,
            width: cmd.width,
            height: 1,
        };
        if chips_h > 0 {
            self.render_chips(frame, line(cmd.y + 1));
        }
        let input_row = line(cmd.y + 1 + chips_h);
        self.input_area = cmd; // a click anywhere in the command region focuses input
        self.render_input(frame, input_row);
    }

    fn render_chips(&mut self, frame: &mut Frame, area: Rect) {
        let mut x = area.x;
        let labels: Vec<String> = self.presets.iter().map(|p| p.label.clone()).collect();
        for (i, label) in labels.iter().enumerate() {
            if x >= area.x + area.width {
                break;
            }
            x = self.chip(
                frame,
                area.y,
                x,
                label,
                Action::LoadPreset(i),
                Color::Indexed(238),
            ) + 1;
        }
    }

    fn chip_width(&self, label: &str) -> u16 {
        if self.icons.pills {
            (Span::raw(self.icons.pl_left).width()
                + Span::raw(format!(" {label} ")).width()
                + Span::raw(self.icons.pl_right).width()) as u16
        } else {
            Span::raw(format!("[{label}]")).width() as u16
        }
    }

    /// Draw a powerline-pill button and register a hit. Returns the end x.
    /// In ascii mode (pills=false) it falls back to `[label]`.
    fn chip(
        &mut self,
        frame: &mut Frame,
        y: u16,
        x: u16,
        label: &str,
        action: Action,
        bg: Color,
    ) -> u16 {
        let (line, w) = if self.icons.pills {
            let mid = format!(" {label} ");
            let w = (Span::raw(self.icons.pl_left).width()
                + Span::raw(&mid).width()
                + Span::raw(self.icons.pl_right).width()) as u16;
            let line = Line::from(vec![
                Span::styled(self.icons.pl_left, Style::default().fg(bg)),
                Span::styled(mid, Style::default().bg(bg).fg(Color::White)),
                Span::styled(self.icons.pl_right, Style::default().fg(bg)),
            ]);
            (line, w)
        } else {
            let text = format!("[{label}]");
            let w = Span::raw(&text).width() as u16;
            (Line::from(Span::styled(text, Style::default().fg(bg))), w)
        };
        let rect = Rect {
            x,
            y,
            width: w,
            height: 1,
        };
        frame.render_widget(Paragraph::new(line), rect);
        self.hit.push((rect, Hit::Fire(action)));
        x + w
    }

    fn render_tree(&mut self, frame: &mut Frame, area: Rect) {
        let cursor = self.tree.cursor;
        let active = self.active_pane == Pane::Tree;
        let visible = self.tree.visible();
        let items: Vec<ListItem> = visible
            .iter()
            .enumerate()
            .map(|(i, row)| {
                let selected = i == cursor;
                let line = match row.node {
                    NodeRef::Repo(id) => self.repo_line(row.indent, id, selected, active),
                    group => self.group_line(row.indent, group, selected, active),
                };
                ListItem::new(line)
            })
            .collect();

        // No border. Selection is drawn as an accent bar ourselves (no highlight_style).
        let list = List::new(items);
        frame.render_stateful_widget(list, area, &mut self.tree_state);

        // Row content starts after the 2-cell selection-bar area.
        let text_x = area.x + SEL_PREFIX;
        let offset = self.tree_state.offset();
        for i in 0..area.height {
            let idx = offset + i as usize;
            let Some(row) = visible.get(idx) else {
                break;
            };
            let rect = Rect {
                x: area.x,
                y: area.y + i,
                width: area.width,
                height: 1,
            };
            self.hit.push((
                rect,
                Hit::TreeRow {
                    node: row.node,
                    row_index: idx,
                    text_x,
                    indent: row.indent,
                },
            ));
        }
    }

    /// Accent bar for the selected row (fixed 2 cells).
    fn sel_prefix(&self, selected: bool, active: bool) -> Span<'static> {
        if selected {
            let color = if active { Color::Cyan } else { Color::DarkGray };
            Span::styled(
                format!("{} ", self.icons.sel_bar),
                Style::default().fg(color),
            )
        } else {
            Span::raw("  ")
        }
    }

    fn repo_line(
        &self,
        indent_level: u16,
        id: RepoId,
        selected: bool,
        active: bool,
    ) -> Line<'static> {
        let repo = &self.repos[id];
        let indent = "  ".repeat(indent_level as usize);
        let cb = if repo.checked {
            self.icons.check_on
        } else {
            self.icons.check_off
        };
        let cb_color = if repo.checked {
            Color::Cyan
        } else {
            Color::DarkGray
        };
        let name_style = Style::default().fg(name_color(repo.status, repo.checked));

        let mut spans = vec![
            self.sel_prefix(selected, active),
            Span::raw(indent),
            Span::styled(cb, Style::default().fg(cb_color)),
            Span::raw(" "),
        ];
        if !self.icons.repo.is_empty() {
            spans.push(Span::styled(
                format!("{} ", self.icons.repo),
                Style::default().fg(Color::DarkGray),
            ));
        }
        spans.push(Span::styled(repo.name.clone(), name_style));
        if self.focus == Some(id) {
            spans.push(Span::styled(
                format!(" {}", self.icons.focus),
                Style::default().fg(Color::Cyan),
            ));
        }
        let status = self.status_span(repo.status);
        if !status.content.is_empty() {
            spans.push(Span::raw(" "));
            spans.push(status);
        }
        Line::from(spans)
    }

    fn group_line(
        &self,
        indent_level: u16,
        group: NodeRef,
        selected: bool,
        active: bool,
    ) -> Line<'static> {
        let indent = "  ".repeat(indent_level as usize);
        let marker = if self.tree.is_expanded(group) {
            self.icons.expanded
        } else {
            self.icons.collapsed
        };
        let (cb, cb_color) = match self.check_state(group) {
            CheckState::All => (self.icons.check_on, Color::Cyan),
            CheckState::Partial => (self.icons.check_partial, Color::Cyan),
            CheckState::None => (self.icons.check_off, Color::DarkGray),
        };
        let icon = match group {
            NodeRef::Fqdn(_) => self.icons.fqdn,
            NodeRef::User(..) => self.icons.user,
            NodeRef::Repo(_) => "",
        };
        let mut spans = vec![
            self.sel_prefix(selected, active),
            Span::raw(indent),
            Span::styled(marker.to_string(), Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
            Span::styled(cb, Style::default().fg(cb_color)),
            Span::raw(" "),
        ];
        if !icon.is_empty() {
            spans.push(Span::styled(
                format!("{icon} "),
                Style::default().fg(Color::DarkGray),
            ));
        }
        spans.push(Span::styled(
            self.tree.group_label(group).to_string(),
            Style::default().add_modifier(Modifier::BOLD),
        ));
        Line::from(spans)
    }

    fn render_output(&mut self, frame: &mut Frame, area: Rect) {
        let border = self.pane_border(Pane::Output);
        // Left rule only = vertical divider from the tree. No outer border or title.
        let block = Block::default().borders(Borders::LEFT).border_style(border);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if self.outputs.is_empty() {
            return; // divider only; render nothing.
        }

        let view_h = inner.height;
        let total = self.output_total() as u16;

        if self.follow {
            self.output_scroll = total.saturating_sub(view_h);
        } else {
            self.output_scroll = self.output_scroll.min(total.saturating_sub(view_h));
        }

        // Build only the visible rows (O(screen height) per frame). No wrap (long lines truncated).
        let lines = self.visible_output(self.output_scroll as usize, view_h as usize);
        frame.render_widget(Paragraph::new(lines), inner);

        // Overlay a scrollbar on the right edge only when content overflows.
        if total > view_h {
            // ratatui's thumb uses (content_length-1)+viewport as the denominator, so
            // pass content_length = (total - view_h) + 1 to make it equal the total line count.
            // Then the thumb reaches the bottom when position is at its max.
            let max_scroll = total - view_h;
            let mut sb_state = ScrollbarState::new(max_scroll as usize + 1)
                .viewport_content_length(view_h as usize)
                .position(self.output_scroll as usize);
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None);
            frame.render_stateful_widget(scrollbar, inner, &mut sb_state);
        }

        // Floating copy button, top-right corner. Drawn last so it sits above the
        // content and the scrollbar; click is resolved before the output-pane fallback.
        let label = self.icons.copy;
        let bw = self.chip_width(label);
        // Right margin so the pill doesn't hug the edge; extra cell when the
        // scrollbar occupies the rightmost column.
        let margin = if total > view_h { 3 } else { 2 };
        let bx = inner.x + inner.width.saturating_sub(bw + margin);
        let bg = match self.copy_flash {
            Some((t, ok)) if t.elapsed() < Duration::from_millis(400) => {
                if ok {
                    Color::Green
                } else {
                    Color::Red
                }
            }
            _ => Color::Indexed(238),
        };
        self.chip(frame, inner.y, bx, label, Action::CopyOutput, bg);
    }

    /// Ordered repo ids shown in the output pane (focus, or all repos with output).
    /// Copy the currently visible output to the system clipboard. Mirrors the
    /// output pane (WYSIWYG): focused repo only, or every repo with output.
    /// Each box becomes a `# user/repo — status` header followed by its raw
    /// lines, with a blank line between boxes.
    fn copy_output(&mut self) {
        let ids = self.output_ids();
        if ids.is_empty() {
            return; // nothing to copy (the button isn't shown when output is empty)
        }

        let mut text = String::new();
        for id in &ids {
            let (label, _) = self.status_pill(*id);
            text.push_str(&format!(
                "# {} — {}\n",
                self.repos[*id].slug(),
                label.trim()
            ));
            for ol in &self.outputs[id].lines {
                text.push_str(&ol.text);
                text.push('\n');
            }
            text.push('\n');
        }

        // Feedback is the button color flash (green ok / red fail), no status text.
        let ok = arboard::Clipboard::new()
            .and_then(|mut cb| cb.set_text(text))
            .is_ok();
        self.copy_flash = Some((Instant::now(), ok));
    }

    fn output_ids(&self) -> Vec<RepoId> {
        match self.focus {
            Some(id) => vec![id],
            None => self
                .repos
                .iter()
                .enumerate()
                .filter(|(id, _)| self.outputs.contains_key(id))
                .map(|(id, _)| id)
                .collect(),
        }
    }

    /// Total line count of the stacked boxes (header + output lines + blank). Counts only, builds no Line.
    fn output_total(&self) -> usize {
        self.output_ids()
            .iter()
            .map(|id| 2 + self.outputs[id].lines.len())
            .sum()
    }

    /// Build only the visible rows in `[start, start+height)`. Out-of-range boxes/lines are
    /// skipped without building a Line, so it stays O(height) per frame even with long output.
    fn visible_output(&self, start: usize, height: usize) -> Vec<Line<'static>> {
        let end = start + height;
        let mut lines = Vec::with_capacity(height);
        let mut idx = 0usize; // global line index

        for id in self.output_ids() {
            let out = &self.outputs[&id];
            let box_len = 2 + out.lines.len();
            let box_end = idx + box_len;
            if box_end <= start {
                idx = box_end; // entirely above
                continue;
            }
            if idx >= end {
                break; // entirely below
            }

            // header（global index = idx）
            if idx >= start && idx < end {
                lines.push(self.box_header(id));
            }

            // Output lines (global index = idx + 1 + i). Skip straight to the first visible one.
            let content_base = idx + 1;
            let first_i = start.saturating_sub(content_base);
            for i in first_i..out.lines.len() {
                let gi = content_base + i;
                if gi >= end {
                    break;
                }
                let ol = &out.lines[i];
                let style = match ol.kind {
                    LineKind::Command => Style::default().fg(Color::DarkGray),
                    LineKind::Stdout => Style::default(),
                    LineKind::Stderr => Style::default().fg(Color::Red),
                };
                lines.push(Line::from(Span::styled(ol.text.clone(), style)));
            }

            // Trailing blank line of the box (global index = box_end - 1)
            let blank_idx = box_end - 1;
            if blank_idx >= start && blank_idx < end {
                lines.push(Line::from(""));
            }

            idx = box_end;
        }
        lines
    }

    /// `row` is the single input line (the rule is drawn by the caller).
    fn render_input(&mut self, frame: &mut Frame, row: Rect) {
        let active = self.active_pane == Pane::Input;
        let prompt_color = if active { Color::Cyan } else { Color::DarkGray };
        let prompt = format!("{} ", self.icons.prompt);
        let prompt_w = Span::raw(&prompt).width() as u16;
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                prompt,
                Style::default()
                    .fg(prompt_color)
                    .add_modifier(Modifier::BOLD),
            ))),
            row,
        );

        // The value renders right of the prompt, horizontally scrolled so the
        // cursor stays visible on long commands. The terminal cursor is placed
        // only while the input pane is active.
        let val_x = row.x + prompt_w;
        let val_w = row.width.saturating_sub(prompt_w);
        let scroll = self.input.visual_scroll(val_w as usize);
        frame.render_widget(
            Paragraph::new(self.input.value()).scroll((0, scroll as u16)),
            Rect {
                x: val_x,
                y: row.y,
                width: val_w,
                height: 1,
            },
        );
        if active {
            let cx = val_x + (self.input.visual_cursor().saturating_sub(scroll)) as u16;
            frame.set_cursor_position((cx, row.y));
        }

        // Right edge: cancel (only while running) → counter / state to its left.
        let mut right_x = row.x + row.width;
        if self.running {
            let w = self.chip_width(self.icons.cancel);
            right_x = right_x.saturating_sub(w);
            self.chip(
                frame,
                row.y,
                right_x,
                self.icons.cancel,
                Action::Cancel,
                Color::Red,
            );
            right_x = right_x.saturating_sub(1);
        }
        let checked = self.repos.iter().filter(|r| r.checked).count();
        let state = if self.running { "running" } else { "idle" };
        let status = match &self.notice {
            Some(msg) => format!("{msg}   {checked} {} · {state}", self.icons.check_on),
            None => format!("{checked} {} · {state}", self.icons.check_on),
        };
        let sw = Span::raw(&status).width() as u16;
        let sx = right_x.saturating_sub(sw);
        frame.render_widget(
            Paragraph::new(status).style(Style::default().fg(Color::DarkGray)),
            Rect {
                x: sx,
                y: row.y,
                width: sw,
                height: 1,
            },
        );
    }

    /// Border color. active = accent, inactive = dim (rules/dividers stay faint).
    fn pane_border(&self, pane: Pane) -> Style {
        let color = if self.active_pane == pane {
            Color::Cyan
        } else {
            Color::DarkGray
        };
        Style::default().fg(color)
    }

    /// Output box header: powerline pill in nerd mode, bar in ascii.
    fn box_header(&self, id: RepoId) -> Line<'static> {
        let name = self.repos[id].slug();
        let (label, color) = self.status_pill(id);

        if !self.icons.pills {
            return Line::from(vec![
                Span::styled(
                    format!("{} {name}  ", self.icons.box_bar),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Span::styled(label, Style::default().fg(color)),
            ]);
        }

        let name_bg = Color::Indexed(238);
        Line::from(vec![
            Span::styled(self.icons.pl_left, Style::default().fg(name_bg)),
            Span::styled(
                format!(" {name} "),
                Style::default()
                    .bg(name_bg)
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(self.icons.pl_sep, Style::default().fg(name_bg).bg(color)),
            Span::styled(
                format!(" {label} "),
                Style::default().bg(color).fg(Color::Black),
            ),
            Span::styled(self.icons.pl_right, Style::default().fg(color)),
        ])
    }

    /// The pill's status label and color.
    fn status_pill(&self, id: RepoId) -> (String, Color) {
        let i = self.icons;
        match self.repos[id].status {
            RepoStatus::Idle => ("idle".into(), Color::DarkGray),
            RepoStatus::Running => {
                let frame = i.spinner[(self.tick / 2) % i.spinner.len()];
                (format!("{frame} running"), Color::Yellow)
            }
            RepoStatus::Done(0) => ("done".into(), Color::Green),
            RepoStatus::Done(code) => (format!("exit {code}"), Color::Red),
            RepoStatus::Failed => ("failed".into(), Color::Red),
            RepoStatus::Cancelled => ("cancelled".into(), Color::Yellow),
            RepoStatus::Skipped => ("skipped".into(), Color::DarkGray),
        }
    }

    /// Render a repo's run state as one span (per icons).
    fn status_span(&self, status: RepoStatus) -> Span<'static> {
        let i = self.icons;
        match status {
            RepoStatus::Idle => Span::raw(""),
            RepoStatus::Running => {
                let frame = i.spinner[(self.tick / 2) % i.spinner.len()];
                Span::styled(frame, Style::default().fg(Color::Yellow))
            }
            RepoStatus::Done(0) => Span::styled(i.status_done, Style::default().fg(Color::Green)),
            RepoStatus::Done(code) => Span::styled(
                format!("{}{code}", i.status_fail),
                Style::default().fg(Color::Red),
            ),
            RepoStatus::Failed => Span::styled(i.status_fail, Style::default().fg(Color::Red)),
            RepoStatus::Cancelled => {
                Span::styled(i.status_cancel, Style::default().fg(Color::Yellow))
            }
            RepoStatus::Skipped => {
                Span::styled(i.status_skip, Style::default().fg(Color::DarkGray))
            }
        }
    }
}

/// Width of the selection-bar area at the start of a tree row (bar + space).
const SEL_PREFIX: u16 = 2;

fn contains(rect: Rect, col: u16, row: u16) -> bool {
    col >= rect.x && col < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height
}

/// Repo name color: by status if any, otherwise brightness by checked.
fn name_color(status: RepoStatus, checked: bool) -> Color {
    match status {
        RepoStatus::Running => Color::Yellow,
        RepoStatus::Done(0) => Color::Green,
        RepoStatus::Done(_) | RepoStatus::Failed => Color::Red,
        RepoStatus::Cancelled => Color::Yellow,
        RepoStatus::Skipped => Color::DarkGray,
        RepoStatus::Idle => {
            if checked {
                Color::Reset
            } else {
                Color::DarkGray
            }
        }
    }
}

/// Pre-check repos matching the `default_checked` globs (`fqdn/user/repo`).
fn apply_default_checked(repos: &mut [Repo], patterns: &[String]) {
    let globs: Vec<glob::Pattern> = patterns
        .iter()
        .filter_map(|p| glob::Pattern::new(p).ok())
        .collect();
    if globs.is_empty() {
        return;
    }
    for repo in repos.iter_mut() {
        let rel = repo.rel_path();
        if globs.iter().any(|g| g.matches(&rel)) {
            repo.checked = true;
        }
    }
}

/// Final keymap with preset.key merged into the config keymap.
fn build_keymap(config: &Config) -> HashMap<String, String> {
    let mut map = config.keymap.clone();
    for (i, preset) in config.presets.iter().enumerate() {
        if let Some(key) = &preset.key {
            map.entry(key.clone())
                .or_insert_with(|| format!("preset:{i}"));
        }
    }
    map
}

/// Normalize a KeyEvent into a keymap lookup string (e.g. `ctrl+r`, `space`, `j`).
fn key_to_string(key: KeyEvent) -> Option<String> {
    let mut s = String::new();
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        s.push_str("ctrl+");
    }
    if key.modifiers.contains(KeyModifiers::ALT) {
        s.push_str("alt+");
    }
    let base = match key.code {
        KeyCode::Char(' ') => "space".to_string(),
        KeyCode::Char(c) => c.to_ascii_lowercase().to_string(),
        KeyCode::Up => "up".to_string(),
        KeyCode::Down => "down".to_string(),
        KeyCode::Left => "left".to_string(),
        KeyCode::Right => "right".to_string(),
        KeyCode::Enter => "enter".to_string(),
        KeyCode::Esc => "esc".to_string(),
        _ => return None,
    };
    s.push_str(&base);
    Some(s)
}

/// Decide the Action from the clicked column of a tree row. repo: checkbox→toggle / else→focus.
/// group: checkbox→toggle / marker or label→toggle expand.
fn tree_click_action(
    tree: &TreeView,
    node: NodeRef,
    indent_level: u16,
    text_x: u16,
    col: u16,
) -> Option<Action> {
    if col < text_x {
        return None; // ignore clicks on the selection-bar area (cursor move only)
    }
    let rel = col - text_x;
    let indent = indent_level * 2; // matches "  ".repeat

    match node {
        NodeRef::Repo(id) => {
            // "{indent}{cb(1)} {icon} {name}"
            if rel == indent {
                Some(Action::ToggleCheck(node))
            } else if rel > indent {
                Some(Action::Focus(id))
            } else {
                None
            }
        }
        group => {
            // "{indent}{marker(1)} {cb(1)} {icon} {label}"
            if rel == indent + 2 {
                Some(Action::ToggleCheck(group))
            } else {
                Some(toggle_expand(tree, group))
            }
        }
    }
}

fn toggle_expand(tree: &TreeView, node: NodeRef) -> Action {
    if tree.is_expanded(node) {
        Action::Collapse(node)
    } else {
        Action::Expand(node)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn dump(app: &mut App, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        let buffer = terminal.backend().buffer();
        let area = *buffer.area();
        let mut s = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                s.push_str(buffer[(x, y)].symbol());
            }
            s.push('\n');
        }
        s
    }

    /// Smoke test that discovers the real ~/src and renders one frame.
    #[test]
    fn render_smoke() {
        let mut app = App::new();
        println!("{}", dump(&mut app, 110, 28));
        println!("--- discovered {} repos ---", app.repos.len());
    }

    fn click(app: &mut App, col: u16, row: u16) {
        app.on_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: col,
            row,
            modifiers: KeyModifiers::empty(),
        });
    }

    /// Whether tree-row checkbox / name clicks work via hit-test.
    #[test]
    fn mouse_click_toggles_checkbox_and_focus() {
        // Default config (no default_checked) so the test doesn't depend on the
        // machine's ~/.config/git-foreach/config.toml leaving repos checked.
        let mut app = App::from_config(Config::default(), None);
        if app.repos.is_empty() {
            return;
        }
        let mut terminal = Terminal::new(TestBackend::new(110, 28)).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();

        // Find the first repo row.
        let (rect, id, text_x, indent) = app
            .hit
            .iter()
            .find_map(|(r, h)| match h {
                Hit::TreeRow {
                    node: NodeRef::Repo(id),
                    text_x,
                    indent,
                    ..
                } => Some((*r, *id, *text_x, *indent)),
                _ => None,
            })
            .expect("a repo row should be hittable");

        // click the checkbox column → check.
        assert!(!app.repos[id].checked);
        click(&mut app, text_x + indent * 2, rect.y);
        assert!(
            app.repos[id].checked,
            "checkbox click should check the repo"
        );

        // click the name column (right of the checkbox) → focus.
        assert_eq!(app.focus, None);
        click(&mut app, text_x + indent * 2 + 5, rect.y);
        assert_eq!(app.focus, Some(id), "name click should focus the repo");
    }

    /// The visible-range slice returns correct total / window / first header.
    #[test]
    fn visible_output_slices_window() {
        let mut app = App::new();
        if app.repos.len() < 2 {
            return;
        }
        let mut o0 = RepoOutput::default();
        o0.push_command("cmd0");
        for i in 0..5 {
            o0.push_stream(crate::runner::Stream::Stdout, format!("a{i}"));
        }
        app.outputs.insert(0, o0); // box0: header+ (1 cmd +5) +blank = 8
        let mut o1 = RepoOutput::default();
        o1.push_command("cmd1");
        app.outputs.insert(1, o1); // box1: header +1 +blank = 3

        assert_eq!(app.output_total(), 11);

        let v = app.visible_output(0, 4);
        assert_eq!(v.len(), 4);
        let head: String = v[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(head.contains(&app.repos[0].slug()));

        // box1 starts at idx 8; even a large window yields only the remaining 3 lines.
        let tail = app.visible_output(8, 10);
        assert_eq!(tail.len(), 3);
    }

    /// Clicking the command field moves focus to the input pane.
    #[test]
    fn click_focuses_input_pane() {
        let mut app = App::new();
        let mut terminal = Terminal::new(TestBackend::new(110, 28)).unwrap();
        terminal.draw(|f| app.render(f)).unwrap();
        assert_eq!(app.active_pane, Pane::Tree);

        let r = app.input_area;
        click(&mut app, r.x + 2, r.y + 1);
        assert_eq!(app.active_pane, Pane::Input);
    }

    /// Run a real command on one repo and check output lands in the box.
    #[test]
    fn run_streams_into_box() {
        let mut app = App::new();
        if app.repos.is_empty() {
            return;
        }
        app.repos[0].checked = true;
        app.input = Input::new("echo wired-up".to_string());
        app.apply(Action::Run);
        assert!(app.running);

        // Drain until done (workers are on separate threads).
        for _ in 0..200 {
            app.drain_events();
            if !app.running {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!app.running, "did not finish in time");

        let out = &app.outputs[&0];
        assert!(out.finished);
        assert_eq!(out.code, Some(0));
        assert!(out.lines.iter().any(|l| l.text == "wired-up"));
    }

    #[test]
    fn history_browses_up_and_down_and_restores_draft() {
        let mut app = App::from_config(Config::default(), None);
        app.history = vec!["a".into(), "b".into(), "c".into()];
        app.input = Input::new("draft".into());

        app.history_prev(); // newest first
        assert_eq!(app.input.value(), "c");
        app.history_prev();
        assert_eq!(app.input.value(), "b");
        app.history_prev();
        assert_eq!(app.input.value(), "a");
        app.history_prev(); // clamps at the oldest
        assert_eq!(app.input.value(), "a");

        app.history_next();
        assert_eq!(app.input.value(), "b");
        app.history_next();
        assert_eq!(app.input.value(), "c");
        app.history_next(); // past the newest → the stashed draft
        assert_eq!(app.input.value(), "draft");
        app.history_next(); // no draft left, nothing to do
        assert_eq!(app.input.value(), "draft");
    }

    #[test]
    fn paste_inserts_at_cursor_and_drops_control_chars() {
        let mut app = App::from_config(Config::default(), None);
        app.active_pane = Pane::Input;
        app.input = Input::new("git ".into());
        app.on_paste("pull\n--rebase".into());
        assert_eq!(app.input.value(), "git pull--rebase");

        // Ignored unless the input pane is active.
        app.active_pane = Pane::Tree;
        app.on_paste(" ignored".into());
        assert_eq!(app.input.value(), "git pull--rebase");
    }
}
