//! Key bindings viewer / rebinder with conflict surfacing.

use ratatui::prelude::*;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::app::Model;

pub fn render(frame: &mut Frame, area: Rect, model: &mut Model) {
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(4),
        Constraint::Length(2),
    ])
    .split(area);

    let blocked = if model.keys.conflicts.is_empty() {
        "ok"
    } else {
        "CONFLICT — fix before save"
    };
    let title = Paragraph::new(format!(
        " wezterminator  ·  keys  ·  {blocked}  ·  r:rebind  ·  s:save  ·  esc:back "
    ))
    .style(Style::new().fg(Color::LightRed));
    frame.render_widget(title, chunks[0]);

    let items: Vec<ListItem> = model
        .keys
        .bindings
        .iter()
        .map(|b| {
            let conflict = model.keys.conflicts.iter().any(|c| c.id == b.id);
            let mark = if conflict { "!" } else { " " };
            let line = Line::from(vec![
                Span::styled(
                    format!("{mark}{:<22}", b.id),
                    if conflict {
                        Style::new().fg(Color::Red)
                    } else {
                        Style::new().fg(Color::Cyan)
                    },
                ),
                Span::raw(format!("  {:<18}  {:<16}  → {}", b.mods, b.key, b.action)),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(" Bindings "))
        .highlight_style(Style::new().fg(Color::Black).bg(Color::LightRed))
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, chunks[1], &mut model.keys.list_state);

    let conflict_text = if model.keys.conflicts.is_empty() {
        "no conflicts".into()
    } else {
        model
            .keys
            .conflicts
            .iter()
            .take(3)
            .map(|c| format!("{}: {}", c.id, c.message))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let conflicts = Paragraph::new(conflict_text).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Conflicts (shown before save) "),
    );
    frame.render_widget(conflicts, chunks[2]);

    let status = Paragraph::new(model.status.as_str())
        .block(Block::default().borders(Borders::TOP).title(" status "));
    frame.render_widget(status, chunks[3]);
}
