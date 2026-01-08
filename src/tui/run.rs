use std::io;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::sync::mpsc;

use crate::tui::app::{App, LibraryFocus, LibraryMode, Screen};
use crate::tui::async_bridge::{AsyncBridge, AsyncRequest, AsyncResponse};
use crate::tui::event::{AppEvent, EventHandler};
use crate::tui::ui;

pub fn run(output_dir: PathBuf) -> Result<()> {
    let (request_tx, request_rx) = mpsc::channel::<AsyncRequest>(32);
    let (response_tx, response_rx) = mpsc::channel::<AsyncResponse>(32);

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(request_tx.clone());
    app.output_dir = output_dir;

    let bridge = AsyncBridge::new(request_rx, response_tx);
    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(bridge.run());
    });

    let event_handler = EventHandler::new(Duration::from_millis(100));

    // Response receiver needs to be checked without blocking
    let mut response_rx = response_rx;

    let result = run_loop(&mut terminal, &mut app, &event_handler, &mut response_rx);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    event_handler: &EventHandler,
    response_rx: &mut mpsc::Receiver<AsyncResponse>,
) -> Result<()> {
    while !app.should_quit {
        // Draw
        terminal.draw(|f| ui::draw(f, app))?;

        // Check for async responses (non-blocking)
        while let Ok(response) = response_rx.try_recv() {
            app.handle_async_response(response);
        }

        // Handle events
        match event_handler.next()? {
            AppEvent::Key(key) => {
                handle_key_event(app, key);
            }
            AppEvent::Tick => {
                app.tick();
            }
            AppEvent::Resize(_, _) => {}
        }
    }

    Ok(())
}

fn handle_key_event(app: &mut App, key: crossterm::event::KeyEvent) {
    use crossterm::event::KeyCode::*;

    // Global quit with Ctrl+C
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == Char('c') {
        app.quit();
        return;
    }

    match app.screen {
        Screen::Login => handle_login_keys(app, key),
        Screen::Library => handle_library_keys(app, key),
        Screen::Download => handle_download_keys(app, key),
    }
}

fn handle_login_keys(app: &mut App, key: crossterm::event::KeyEvent) {
    use crossterm::event::KeyCode::*;

    if app.login_state.loading {
        return; // Ignore input while loading
    }

    match key.code {
        Char('q') => app.quit(),
        Char(c) => app.login_input_char(c),
        Backspace => app.login_delete_char(),
        Enter => app.login_submit(),
        _ => {}
    }
}

fn handle_library_keys(app: &mut App, key: crossterm::event::KeyEvent) {
    use crossterm::event::KeyCode::*;

    if app.library_state.loading {
        return; // Ignore input while loading
    }

    match app.library_state.mode {
        LibraryMode::Browse => {
            // Tab toggles focus between search bar and list
            if key.code == Tab {
                app.library_toggle_focus();
                return;
            }

            match app.library_state.focus {
                LibraryFocus::SearchBar => match key.code {
                    Char(c) => app.library_search_input(c),
                    Backspace => app.library_search_backspace(),
                    Esc => app.library_search_clear(),
                    // Arrow keys navigate list even while in search
                    Down => app.library_move_down(),
                    Up => app.library_move_up(),
                    Enter => {
                        // Enter in search bar focuses the list
                        app.library_focus_list();
                    }
                    _ => {}
                },
                LibraryFocus::List => match key.code {
                    Char('q') => app.quit(),
                    Down | Char('j') => app.library_move_down(),
                    Up | Char('k') => app.library_move_up(),
                    Enter | Char(' ') => app.library_toggle_selection(),
                    Char('a') => app.library_select_all(),
                    Char('n') => app.library_clear_selection(),
                    Char('d') => app.library_show_format_selection(),
                    Char('/') => app.library_focus_search(),
                    Esc => {
                        if !app.library_state.search_query.is_empty() {
                            app.library_search_clear();
                        } else {
                            app.library_clear_selection();
                        }
                    }
                    _ => {}
                },
            }
        }
        LibraryMode::FormatSelection => match key.code {
            Char('q') | Esc => app.library_cancel_format_selection(),
            Char('j') | Down => app.format_move_down(),
            Char('k') | Up => app.format_move_up(),
            Enter => app.format_confirm(),
            _ => {}
        },
    }
}

fn handle_download_keys(app: &mut App, key: crossterm::event::KeyEvent) {
    use crossterm::event::KeyCode::*;

    match key.code {
        Char('q') => {
            if !app.download_state.is_active {
                app.quit();
            }
        }
        Enter => app.download_back_to_library(),
        _ => {}
    }
}
