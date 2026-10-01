//! Presets browser screen.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::app::{Model, PreviewMode};

pub fn render(frame: &mut Frame, area: Rect, model: &mut Model) {
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(2),
    ])
    .split(area);

    let mode = match model.mode {
        PreviewMode::WezTerm => "wezterm",
        PreviewMode::Browser => "browser",
    };
    let title = Paragraph::new(format!(
        " wezterminator  ·  presets  ·  preview:{mode}  ·  tab:parts  ·  A/K/M/F  ·  enter:commit  ·  esc:quit "
    ))
    .style(Style::new().fg(Color::Cyan));
    frame.render_widget(title, chunks[0]);

    let items: Vec<ListItem> = model
        .presets
        .iter()
        .map(|row| {
            let marker = if row.shadowed { "·" } else { " " };
            let active = if model.active_id.as_deref() == Some(row.id.as_str()) {
                "*"
            } else {
                " "
            };
            let line = format!(
                "{active}{marker} {:<22}  {:<16}  [{}]",
                row.name,
                row.id,
                row.layer.as_str()
            );
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(" Presets "))
        .highlight_style(Style::new().fg(Color::Black).bg(Color::Cyan))
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, chunks[1], &mut model.preset_state);

    let status = Paragraph::new(model.status.as_str())
        .block(Block::default().borders(Borders::TOP).title(" status "));
    frame.render_widget(status, chunks[2]);
}
