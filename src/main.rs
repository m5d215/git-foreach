mod action;
mod app;
mod config;
mod output;
mod repo;
mod runner;
mod theme;
mod tree;

use std::io::{self, Stdout};
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use app::App;

type Tui = Terminal<CrosstermBackend<Stdout>>;

fn setup_terminal() -> Result<Tui> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    Ok(Terminal::new(CrosstermBackend::new(stdout))?)
}

fn restore_terminal() -> Result<()> {
    execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;
    disable_raw_mode()?;
    Ok(())
}

/// Restore the terminal on panic so it is never left in raw / alt-screen mode.
fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = restore_terminal();
        original(info);
    }));
}

fn main() -> Result<()> {
    install_panic_hook();
    let mut terminal = setup_terminal()?;
    let result = run(&mut terminal);
    restore_terminal()?;
    result
}

fn run(terminal: &mut Tui) -> Result<()> {
    let mut app = App::new();
    while !app.should_quit {
        app.drain_events();
        terminal.draw(|frame| app.render(frame))?;

        if event::poll(Duration::from_millis(100))? {
            // Drain all queued input before redrawing. Redrawing once per event
            // would flood redraws/flushes on bursts (e.g. fast scroll wheels) and freeze.
            loop {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => app.on_key(key),
                    Event::Mouse(mouse) => app.on_mouse(mouse),
                    Event::Resize(_, _) => {}
                    _ => {}
                }
                if app.should_quit || !event::poll(Duration::ZERO)? {
                    break;
                }
            }
        }
    }
    Ok(())
}
