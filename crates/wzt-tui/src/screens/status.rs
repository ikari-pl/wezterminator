//! Status-bar style and segment order editor.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::app::Model;

pub fn render(frame: &mut Frame, area: Rect, model: &mut Model) {
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(3),
    ])
    .split(area);

    let layer = model.save_layer.label();
    let style = if model.status_ed.style_pill {
        "pill"
    } else {
        "sparkline"
    };
    let title = Paragraph::new(format!(
        " wezterminator  ·  status ({style})  ·  save:{layer}  ·  t:toggle-style  ·  s:save-local  ·  esc:back "
    ))
    .style(Style::new().fg(Color::Magenta));
    frame.render_widget(title, chunks[0]);

    let items: Vec<ListItem> = model
        .status_ed
        .segments
        .iter()
        .enumerate()
        .map(|(i, seg)| {
            let mark = if seg.enabled { "[x]" } else { "[ ]" };
            let unavail = if seg.unavailable {
                "  (unavailable)"
            } else {
                ""
            };
            ListItem::new(format!(" {:>2}. {mark} {}{unavail}", i + 1, seg.id))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Segments (space:toggle  s:save as local preset) "),
        )
        .highlight_style(Style::new().fg(Color::Black).bg(Color::Magenta))
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, chunks[1], &mut model.status_ed.list_state);

    let status = Paragraph::new(model.status.as_str())
        .block(Block::default().borders(Borders::TOP).title(" status "));
    frame.render_widget(status, chunks[2]);
}
