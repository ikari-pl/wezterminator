//! Fonts: installed list, preferred marks, size + per-font corrections.

use ratatui::prelude::*;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::app::Model;

pub fn render(frame: &mut Frame, area: Rect, model: &mut Model) {
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(3),
        Constraint::Min(3),
        Constraint::Length(3),
    ])
    .split(area);

    let layer = model.save_layer.label();
    let title = Paragraph::new(format!(
        " wezterminator  ·  fonts  ·  save:{layer}  ·  [/]:size  ·  c:correction  ·  s:save  ·  esc:back "
    ))
    .style(Style::new().fg(Color::Yellow));
    frame.render_widget(title, chunks[0]);

    let f = &model.fonts;
    let header = Paragraph::new(format!(
        " base size: {:.1} pt   preferred marked with *   ! = missing   p = polish gap   n = nerd gap ",
        f.base_size
    ))
    .block(Block::default().borders(Borders::ALL).title(" Size "));
    frame.render_widget(header, chunks[1]);

    let items: Vec<ListItem> = f
        .rows
        .iter()
        .map(|row| {
            let star = if row.preferred { "*" } else { " " };
            let miss = if row.installed { " " } else { "!" };
            let polish = match row.polish_ok {
                Some(true) => " ",
                Some(false) => "p",
                None => "·",
            };
            let nerd = match row.nerd_ok {
                Some(true) => " ",
                Some(false) => "n",
                None => "·",
            };
            let corr = row
                .correction
                .map(|c| format!("  corr:{c:+.1}"))
                .unwrap_or_default();
            let line = Line::from(vec![
                Span::raw(format!("{star}{miss}{polish}{nerd} ")),
                Span::raw(format!("{:<40}", row.family)),
                Span::styled(corr, Style::new().fg(Color::DarkGray)),
            ]);
            ListItem::new(line)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(" Families "))
        .highlight_style(Style::new().fg(Color::Black).bg(Color::Yellow))
        .highlight_symbol("> ");
    frame.render_stateful_widget(list, chunks[2], &mut model.fonts.list_state);

    let status = Paragraph::new(model.status.as_str())
        .block(Block::default().borders(Borders::TOP).title(" status "));
    frame.render_widget(status, chunks[3]);
}
