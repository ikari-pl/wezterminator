//! Chrome editor: opacity, blur, padding, inactive pane, tab bar.

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
    let title = Paragraph::new(format!(
        " wezterminator  ·  chrome  ·  save:{layer}  ·  j/k:field  ·  [/]:adjust  ·  s:save  ·  esc:back "
    ))
    .style(Style::new().fg(Color::Cyan));
    frame.render_widget(title, chunks[0]);

    let c = &model.chrome;
    let items = [
        format!("opacity          {:.2}", c.opacity),
        format!(
            "blur             {}{}",
            c.blur,
            if c.blur_unavailable {
                "  [unavailable on this platform]"
            } else {
                ""
            }
        ),
        format!("padding L/R/T/B  {}/{}/{}/{}", c.pad_l, c.pad_r, c.pad_t, c.pad_b),
        format!(
            "inactive sat/bri {:.2}/{:.2}",
            c.inactive_sat, c.inactive_bri
        ),
        format!(
            "tab bar          {}  hidden={}",
            if c.tab_top { "top" } else { "bottom" },
            c.tab_hidden
        ),
    ];
    let list_items: Vec<ListItem> = items.into_iter().map(ListItem::new).collect();
    let list = List::new(list_items)
        .block(Block::default().borders(Borders::ALL).title(" Chrome "))
        .highlight_style(Style::new().fg(Color::Black).bg(Color::Cyan))
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, chunks[1], &mut model.chrome.list_state);

    let status = Paragraph::new(model.status.as_str())
        .block(Block::default().borders(Borders::TOP).title(" status "));
    frame.render_widget(status, chunks[2]);
}
