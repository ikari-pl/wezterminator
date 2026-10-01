//! Fonts: installed list, preferred marks, size + per-font corrections.

use ratatui::prelude::*;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use wzt_fonts::FontCatalog;

use crate::app::{FontRow, FontsState, Model};

/// Mark preferred rows against an installed-family catalog (R20).
pub fn apply_catalog(state: &mut FontsState, catalog: &FontCatalog) {
    for row in &mut state.rows {
        row.installed = catalog.contains(&row.family);
    }
}

/// Build rows from preferred names plus catalog extras.
pub fn rows_from_preferred(preferred: &[String], catalog: &FontCatalog) -> Vec<FontRow> {
    let mut rows: Vec<FontRow> = preferred
        .iter()
        .map(|family| FontRow {
            family: family.clone(),
            preferred: true,
            installed: catalog.contains(family),
            polish_ok: None,
            nerd_ok: None,
            correction: None,
        })
        .collect();
    for family in &catalog.families {
        if !rows.iter().any(|r| r.family.eq_ignore_ascii_case(family)) {
            rows.push(FontRow {
                family: family.clone(),
                preferred: false,
                installed: true,
                polish_ok: None,
                nerd_ok: None,
                correction: None,
            });
        }
    }
    rows
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_marks_missing_preferred() {
        let catalog = FontCatalog::from_families(["Menlo"]);
        let mut state = FontsState::default();
        apply_catalog(&mut state, &catalog);
        let terminess = state
            .rows
            .iter()
            .find(|r| r.family.contains("Terminess"))
            .unwrap();
        assert!(!terminess.installed);
        let menlo = state.rows.iter().find(|r| r.family == "Menlo").unwrap();
        assert!(menlo.installed);
    }
}
