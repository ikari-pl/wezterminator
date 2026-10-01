//! Theme-parts overview screen (shell for U12; deep editors in U13).

use ratatui::prelude::*;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::app::Model;

pub fn render(frame: &mut Frame, area: Rect, model: &mut Model) {
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(2),
    ])
    .split(area);

    let preset = model
        .selected_preset()
        .map(|p| p.id.as_str())
        .unwrap_or("(none)");
    let addon = if model.addon_mode { "add-on" } else { "replace" };
    let title = Paragraph::new(format!(
        " wezterminator  ·  parts of {preset}  ·  mode:{addon}  ·  tab:presets  ·  esc:back "
    ))
    .style(Style::new().fg(Color::Yellow));
    frame.render_widget(title, chunks[0]);

    let items: Vec<ListItem> = model
        .parts
        .iter()
        .map(|row| {
            let mark = if row.overruled { "!" } else { " " };
            let line = Line::from(vec![
                Span::raw(format!("{mark} ")),
                Span::styled(
                    format!("{:<8}", row.kind.label()),
                    Style::new().fg(Color::Cyan),
                ),
                Span::raw(format!("  {:<40}  ", row.summary)),
                Span::styled(
                    format!("src:{}", row.source),
                    Style::new().fg(Color::DarkGray),
                ),
                if row.overruled {
                    Span::styled("  [overruled]", Style::new().fg(Color::Red))
                } else {
                    Span::raw("")
                },
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Theme parts (! = overruled by user config in add-on mode) "),
        )
        .highlight_style(Style::new().fg(Color::Black).bg(Color::Yellow))
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, chunks[1], &mut model.part_state);

    let status = Paragraph::new(model.status.as_str())
        .block(Block::default().borders(Borders::TOP).title(" status "));
    frame.render_widget(status, chunks[2]);
}
