//! Motion: parallax, ALT+wheel, auto-scroll.

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
        " wezterminator  ·  motion  ·  save:{layer}  ·  space:toggle  ·  [/]:speed  ·  s:save  ·  esc:back "
    ))
    .style(Style::new().fg(Color::Green));
    frame.render_widget(title, chunks[0]);

    let m = &model.motion;
    let items = [
        format!(
            "scrollback parallax   {}",
            on_off(m.scrollback_parallax)
        ),
        format!("ALT+wheel vertical    {}", on_off(m.alt_vertical)),
        format!("ALT+wheel horizontal  {}", on_off(m.alt_horizontal)),
        format!("auto-scroll           {}", on_off(m.auto_scroll)),
        format!("auto-scroll speed     {:.1} px/tick", m.auto_speed),
        format!(
            "auto-scroll axis      {}",
            if m.auto_horizontal {
                "horizontal"
            } else {
                "vertical"
            }
        ),
    ];
    let list_items: Vec<ListItem> = items.into_iter().map(ListItem::new).collect();
    let list = List::new(list_items)
        .block(Block::default().borders(Borders::ALL).title(" Motion "))
        .highlight_style(Style::new().fg(Color::Black).bg(Color::Green))
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, chunks[1], &mut model.motion.list_state);

    let status = Paragraph::new(model.status.as_str())
        .block(Block::default().borders(Borders::TOP).title(" status "));
    frame.render_widget(status, chunks[2]);
}

fn on_off(v: bool) -> &'static str {
    if v { "on" } else { "off" }
}
