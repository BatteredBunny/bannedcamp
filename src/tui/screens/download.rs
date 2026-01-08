use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

use crate::{
    core::utils::{format_bytes, truncate_str},
    tui::app::DownloadState,
};

pub fn draw(frame: &mut Frame, area: Rect, state: &DownloadState) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray))
        .title(" Downloads ");

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);

    draw_download_list(frame, chunks[0], state);
    draw_help_bar(frame, chunks[1], state);
}

fn draw_download_list(frame: &mut Frame, area: Rect, state: &DownloadState) {
    let mut items: Vec<ListItem> = Vec::new();

    for item in &state.items {
        let display_name = format!("{} - {}", item.artist, item.title);

        if let Some(slot) = state
            .slots
            .iter()
            .find(|s| s.item_id.as_deref() == Some(&item.id))
        {
            items.push(create_progress_item(&display_name, slot, area.width));
            continue;
        }

        if let Some(result) = state.results.iter().find(|r| r.item_id == item.id) {
            let (icon, style, suffix) = match &result.result {
                Ok(_) => ("✓", Style::default().fg(Color::Green), String::new()),
                Err(e) => ("✗", Style::default().fg(Color::Red), format!(" - {}", e)),
            };

            let max_len = area.width.saturating_sub(4) as usize;
            let full_text = format!("{display_name}{suffix}");
            let truncated = truncate_str(&full_text, max_len);

            items.push(ListItem::new(Line::from(vec![
                Span::styled(format!("{icon} "), style),
                Span::styled(truncated, style),
            ])));
            continue;
        }

        let max_len = area.width.saturating_sub(4) as usize;
        let truncated = truncate_str(&display_name, max_len);
        items.push(ListItem::new(Line::from(vec![
            Span::styled("○ ", Style::default().fg(Color::DarkGray)),
            Span::styled(truncated, Style::default().fg(Color::DarkGray)),
        ])));
    }

    let list = List::new(items);
    frame.render_widget(list, area);
}

fn create_progress_item(
    name: &str,
    slot: &crate::tui::app::DownloadSlot,
    width: u16,
) -> ListItem<'static> {
    let progress = slot.progress_percent() as u16;
    let bar_width = 20usize;
    let filled = (progress as usize * bar_width / 100).min(bar_width);
    let empty = bar_width.saturating_sub(filled);

    let progress_bar = format!("[{}{}]", "=".repeat(filled), " ".repeat(empty));

    let speed_str = if slot.speed_bytes_per_sec > 0.0 {
        format!("{:>10}/s", format_bytes(slot.speed_bytes_per_sec as u64))
    } else {
        " ".repeat(13)
    };

    let percent_str = format!("{progress:>4}%");

    let right_width = 1 + (bar_width + 2) + 5 + 13;
    let name_width = (width as usize).saturating_sub(right_width);

    let name_display = if name.chars().count() > name_width {
        truncate_str(name, name_width)
    } else {
        format!("{name:<name_width$}")
    };

    let line = Line::from(vec![
        Span::styled(name_display, Style::default().fg(Color::Cyan)),
        Span::raw(" "),
        Span::styled(progress_bar, Style::default().fg(Color::Blue)),
        Span::styled(percent_str, Style::default().fg(Color::DarkGray)),
        Span::styled(speed_str, Style::default().fg(Color::DarkGray)),
    ]);

    ListItem::new(line)
}

fn draw_help_bar(frame: &mut Frame, area: Rect, state: &DownloadState) {
    let help_text = if state.is_active {
        Line::from(vec![Span::styled(
            "Downloading...",
            Style::default().fg(Color::DarkGray),
        )])
    } else {
        Line::from(vec![
            Span::styled("Enter", Style::default().fg(Color::Yellow)),
            Span::raw(" Back to library "),
            Span::styled("q", Style::default().fg(Color::Yellow)),
            Span::raw(" Quit"),
        ])
    };

    let help = Paragraph::new(help_text).style(Style::default().fg(Color::DarkGray));
    frame.render_widget(help, area);
}
