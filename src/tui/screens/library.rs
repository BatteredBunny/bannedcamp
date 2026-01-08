use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};

use crate::core::library::AudioFormat;
use crate::core::utils::truncate_str;
use crate::tui::app::{LibraryFocus, LibraryMode, LibraryState};

pub fn draw(frame: &mut Frame, area: Rect, state: &LibraryState) {
    // Main layout
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // Search bar
            Constraint::Length(2), // Header
            Constraint::Min(5),    // List
            Constraint::Length(2), // Help bar
        ])
        .split(area);

    // Search bar
    let search_focused = state.focus == LibraryFocus::SearchBar;
    let search_border_color = if search_focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };
    let cursor = if search_focused { "▌" } else { "" };

    let search_bar = Paragraph::new(Line::from(vec![
        Span::styled(" ", Style::default().fg(Color::Yellow)),
        Span::styled(&state.search_query, Style::default().fg(Color::White)),
        Span::styled(cursor, Style::default().fg(Color::Cyan)),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(search_border_color))
            .title(" Search (/) "),
    );
    frame.render_widget(search_bar, chunks[0]);

    // Header with counts
    let selected_count = state.selected_items.len();
    let visible_count = state.visible_count();
    let total_count = state.items.len();

    let (header_text, header_style) = if let Some(ref error) = state.error {
        (
            format!("Error: {}", error),
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )
    } else if state.loading {
        (
            format!("{} Loading library...", state.spinner.current()),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
    } else if !state.search_query.is_empty() {
        let text = if selected_count > 0 {
            format!(
                "Showing {}/{} items ({} selected)",
                visible_count, total_count, selected_count
            )
        } else {
            format!("Showing {}/{} items", visible_count, total_count)
        };
        (
            text,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
    } else if selected_count > 0 {
        (
            format!("{} items ({} selected)", total_count, selected_count),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        (
            format!("{} items", total_count),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
    };

    let header = Paragraph::new(header_text).style(header_style);
    frame.render_widget(header, chunks[1]);

    // Library list
    let list_area = chunks[2];
    let visible_height = list_area.height.saturating_sub(2) as usize; // Account for borders

    // Adjust scroll offset if selected item is out of view
    let scroll_offset = if state.selected >= state.scroll_offset + visible_height {
        state.selected.saturating_sub(visible_height - 1)
    } else if state.selected < state.scroll_offset {
        state.selected
    } else {
        state.scroll_offset
    };

    let visible_items = state.visible_items();
    let items: Vec<ListItem> = visible_items
        .iter()
        .enumerate()
        .skip(scroll_offset)
        .take(visible_height)
        .map(|(display_idx, (_, item))| {
            let is_highlighted = display_idx == state.selected;
            let is_selected = state.selected_items.contains(&item.id);

            // Truncate artist and title to fit
            let artist = truncate_str(&item.artist, 25);
            let title = truncate_str(&item.title, 40);

            let line = format!("{:<25} - {}", artist, title);

            let style = if is_highlighted {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else if is_selected {
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            // Checkbox style prefix
            let checkbox = if is_selected { "[x] " } else { "[ ] " };
            let prefix = if is_highlighted { "▶" } else { " " };

            ListItem::new(Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(checkbox, style),
                Span::styled(line, style),
            ]))
        })
        .collect();

    let list_title = if visible_items.is_empty() {
        if state.search_query.is_empty() {
            " 0/0 ".to_string()
        } else {
            " No matches ".to_string()
        }
    } else {
        format!(" {}/{} ", state.selected + 1, visible_items.len())
    };

    let list_focused = state.focus == LibraryFocus::List;
    let list_border_color = if list_focused {
        Color::Cyan
    } else {
        Color::DarkGray
    };

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(list_border_color))
            .title(list_title),
    );
    frame.render_widget(list, list_area);

    // Help bar - show different hints based on focus
    let help = if state.focus == LibraryFocus::SearchBar {
        Paragraph::new(Line::from(vec![
            Span::styled("Tab", Style::default().fg(Color::Yellow)),
            Span::raw(" List  "),
            Span::styled("Enter", Style::default().fg(Color::Yellow)),
            Span::raw(" Done  "),
            Span::styled("Esc", Style::default().fg(Color::Yellow)),
            Span::raw(" Clear"),
        ]))
    } else {
        Paragraph::new(Line::from(vec![
            Span::styled("j/k", Style::default().fg(Color::Yellow)),
            Span::raw(" Nav  "),
            Span::styled("Space/Enter", Style::default().fg(Color::Yellow)),
            Span::raw(" Select  "),
            Span::styled("a", Style::default().fg(Color::Yellow)),
            Span::raw(" All  "),
            Span::styled("/", Style::default().fg(Color::Yellow)),
            Span::raw(" Search  "),
            Span::styled("d", Style::default().fg(Color::Yellow)),
            Span::raw(" Download  "),
            Span::styled("q", Style::default().fg(Color::Yellow)),
            Span::raw(" Quit"),
        ]))
    }
    .style(Style::default().fg(Color::DarkGray));
    frame.render_widget(help, chunks[3]);

    // Draw format selection popup if active
    if state.mode == LibraryMode::FormatSelection {
        draw_format_selection(frame, area, state);
    }
}

fn draw_format_selection(frame: &mut Frame, area: Rect, state: &LibraryState) {
    // Calculate popup size and position
    let popup_width = 40;
    let popup_height = AudioFormat::ALL.len() as u16 + 4; // formats + border + title + help

    let popup_area = centered_rect(popup_width, popup_height, area);

    // Clear the background
    frame.render_widget(Clear, popup_area);

    // Draw popup block
    let block = Block::default()
        .title(" Select Format ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan))
        .style(Style::default().bg(Color::Black));

    let inner_area = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    // Layout for formats and help
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),    // Format list
            Constraint::Length(1), // Help
        ])
        .split(inner_area);

    // Format list
    let formats: Vec<ListItem> = AudioFormat::ALL
        .iter()
        .enumerate()
        .map(|(i, format)| {
            let is_selected = i == state.selected_format;
            let style = if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Cyan)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let prefix = if is_selected { "▶ " } else { "  " };
            ListItem::new(Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(format.display_name(), style),
            ]))
        })
        .collect();

    let list = List::new(formats);
    frame.render_widget(list, chunks[0]);

    // Help
    let help = Paragraph::new(Line::from(vec![
        Span::styled("Enter", Style::default().fg(Color::Yellow)),
        Span::raw(" Confirm  "),
        Span::styled("Esc", Style::default().fg(Color::Yellow)),
        Span::raw(" Cancel"),
    ]))
    .style(Style::default().fg(Color::DarkGray))
    .alignment(Alignment::Center);
    frame.render_widget(help, chunks[1]);
}

// CSS reference
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}
