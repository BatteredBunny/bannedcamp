use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders},
};

use super::app::{App, Screen};
use super::screens;

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();

    // Main layout with status bar
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),    // Main content
            Constraint::Length(1), // Status bar
        ])
        .split(area);

    // Outer frame
    let main_block = Block::default()
        .title(" Bandcamp Downloader ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let inner_area = main_block.inner(chunks[0]);
    frame.render_widget(main_block, chunks[0]);

    // Draw current screen
    match app.screen {
        Screen::Login => screens::login::draw(frame, inner_area, &app.login_state),
        Screen::Library => screens::library::draw(frame, inner_area, &app.library_state),
        Screen::Download => screens::download::draw(frame, inner_area, &app.download_state),
    }
}
