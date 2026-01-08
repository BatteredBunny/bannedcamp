use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use crate::tui::app::LoginState;

pub fn draw(frame: &mut Frame, area: Rect, state: &LoginState) {
    // Main layout
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(2)
        .constraints([
            Constraint::Length(3), // Title
            Constraint::Length(2), // Spacer
            Constraint::Length(4), // Instructions
            Constraint::Length(3), // Input field
            Constraint::Length(2), // Spacer
            Constraint::Min(3),    // Status/error
        ])
        .split(area);

    // Title
    let title = Paragraph::new("Bandcamp Downloader")
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .alignment(ratatui::layout::Alignment::Center);
    frame.render_widget(title, chunks[0]);

    // Instructions
    let env_prefilled = std::env::var("BANDCAMP_COOKIE").is_ok();
    let instructions = if state.loading {
        vec![Line::from(vec![
            Span::raw(state.spinner.current()),
            Span::raw(" Validating cookie..."),
        ])]
    } else if env_prefilled && !state.cookie_input.is_empty() {
        vec![
            Line::from(Span::styled(
                "Cookie loaded from BANDCAMP_COOKIE",
                Style::default().fg(Color::Green),
            )),
            Line::from(""),
            Line::from("Press Enter to continue"),
        ]
    } else {
        vec![
            Line::from("Paste your Bandcamp identity cookie below:"),
            Line::from(""),
            Line::from(Span::styled(
                "(Browser DevTools -> Application -> Cookies -> identity)",
                Style::default().fg(Color::DarkGray),
            )),
        ]
    };
    let instructions = Paragraph::new(instructions).alignment(ratatui::layout::Alignment::Center);
    frame.render_widget(instructions, chunks[2]);

    // Input field
    let input_width = area.width.saturating_sub(6);
    let input_x = area.x + (area.width.saturating_sub(input_width)) / 2;
    let input_area = Rect::new(input_x, chunks[3].y, input_width, 3);

    let input_style = if state.loading {
        Style::default().fg(Color::DarkGray)
    } else {
        Style::default()
    };

    let display_text = state.cookie_input.clone();

    let display_len = display_text.chars().count();
    let input = Paragraph::new(display_text).style(input_style).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(if state.loading {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default().fg(Color::Cyan)
            })
            .title(" Cookie "),
    );
    frame.render_widget(input, input_area);

    // Show cursor if not loading
    if !state.loading {
        let cursor_x = input_area.x + 1 + (display_len as u16).min(input_width - 2);
        frame.set_cursor_position((cursor_x, input_area.y + 1));
    }

    // Error or hint
    let status_text = if let Some(ref error) = state.error {
        vec![Line::from(Span::styled(
            error.as_str(),
            Style::default().fg(Color::Red),
        ))]
    } else if state.loading {
        vec![]
    } else {
        vec![
            Line::from(""),
            Line::from(Span::styled(
                "Enter Submit  q Quit",
                Style::default().fg(Color::DarkGray),
            )),
        ]
    };
    let status = Paragraph::new(status_text).alignment(ratatui::layout::Alignment::Center);
    frame.render_widget(status, chunks[5]);
}
