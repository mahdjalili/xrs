mod app;
mod input;
mod tasks;
mod view;

use app::App;
use crossterm::{
    event::{self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture},
    execute,
};
use ratatui::DefaultTerminal;
use std::io;
use std::time::Duration;

/// Redraw cadence while a spinner is visible.
const FRAME: Duration = Duration::from_millis(80);
/// Redraw cadence otherwise; matches the connection status refresh interval.
const IDLE_FRAME: Duration = Duration::from_millis(500);

pub fn run_tui() -> Result<(), Box<dyn std::error::Error>> {
    let mut terminal = ratatui::try_init()?;
    execute!(io::stdout(), EnableMouseCapture, EnableBracketedPaste)?;

    // ratatui's hook restores raw mode and the alternate screen; mouse capture
    // and bracketed paste must be undone too or the user's shell is left broken.
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(io::stdout(), DisableBracketedPaste, DisableMouseCapture);
        previous(info);
    }));

    let result = run(&mut terminal);

    let _ = execute!(io::stdout(), DisableBracketedPaste, DisableMouseCapture);
    ratatui::try_restore()?;
    result
}

fn run(terminal: &mut DefaultTerminal) -> Result<(), Box<dyn std::error::Error>> {
    let mut app = App::new();
    while !app.should_quit {
        app.tick();
        terminal.draw(|f| view::render(&mut app, f))?;
        let timeout = if app.animating() { FRAME } else { IDLE_FRAME };
        if event::poll(timeout)? {
            app.on_event(event::read()?);
            while event::poll(Duration::ZERO)? {
                app.on_event(event::read()?);
            }
        }
    }
    Ok(())
}
