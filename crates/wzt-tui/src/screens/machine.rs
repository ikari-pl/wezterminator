//! Machine settings editor (local layer only — never in presets).

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

    let title = Paragraph::new(
        " wezterminator  ·  machine  ·  local layer only  ·  s:save  ·  esc:back ",
    )
    .style(Style::new().fg(Color::Blue));
    frame.render_widget(title, chunks[0]);

    let m = &model.machine;
    let items = [
        format!("project_roots       {}", m.project_roots),
        format!("issue_url_pattern   {}", m.issue_url_pattern),
        format!("issue_key_pattern   {}", m.issue_key_pattern),
        format!("editor              {}", m.editor),
        format!("vpn_probes          {}", m.vpn_summary),
        format!("push_targets        {}", m.push_summary),
        format!("dev_art_path        {}", m.dev_art_path),
        format!("screen_overrides    {}", m.screen_overrides),
    ];
    let list_items: Vec<ListItem> = items.into_iter().map(ListItem::new).collect();
    let list = List::new(list_items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Machine settings (not exported with presets) "),
        )
        .highlight_style(Style::new().fg(Color::Black).bg(Color::Blue))
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, chunks[1], &mut model.machine.list_state);

    let status = Paragraph::new(model.status.as_str())
        .block(Block::default().borders(Borders::TOP).title(" status "));
    frame.render_widget(status, chunks[2]);
}
