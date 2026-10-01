//! Theme authoring: template/duplicate, palette + contrast, art regen, import.

use ratatui::prelude::*;
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::app::{AuthorTab, Model};

pub fn render(frame: &mut Frame, area: Rect, model: &mut Model) {
    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(4),
        Constraint::Length(2),
    ])
    .split(area);

    let title = Paragraph::new(
        " wezterminator  ·  author  ·  1:theme 2:palette 3:art 4:import  ·  s:save  ·  esc:back ",
    )
    .style(Style::new().fg(Color::LightMagenta));
    frame.render_widget(title, chunks[0]);

    let tab = match model.author.tab {
        AuthorTab::Theme => "theme",
        AuthorTab::Palette => "palette",
        AuthorTab::Art => "art",
        AuthorTab::Import => "import",
    };
    let tab_line = Paragraph::new(format!(" tab: {tab}   theme: {}", model.author.theme_name))
        .style(Style::new().fg(Color::Gray));
    frame.render_widget(tab_line, chunks[1]);

    match model.author.tab {
        AuthorTab::Theme => {
            let items = [
                format!("name / id     {}  ({})", model.author.theme_name, model.author.theme_id),
                "action        n:new-from-template   d:duplicate-current".into(),
                format!("variant       {}", model.author.variant),
            ];
            let list_items: Vec<ListItem> = items.into_iter().map(ListItem::new).collect();
            let list = List::new(list_items)
                .block(Block::default().borders(Borders::ALL).title(" Theme "))
                .highlight_style(Style::new().fg(Color::Black).bg(Color::LightMagenta))
                .highlight_symbol("> ");
            frame.render_stateful_widget(list, chunks[2], &mut model.author.list_state);
        }
        AuthorTab::Palette => {
            let items: Vec<ListItem> = model
                .author
                .palette_rows
                .iter()
                .map(|row| {
                    let warn = if row.contrast_ok { " " } else { "!" };
                    ListItem::new(format!("{warn} {:<18}  {}", row.key, row.hex))
                })
                .collect();
            let list = List::new(items)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(" Palette (! = below contrast; save still allowed) "),
                )
                .highlight_style(Style::new().fg(Color::Black).bg(Color::LightMagenta))
                .highlight_symbol("> ");
            frame.render_stateful_widget(list, chunks[2], &mut model.author.list_state);
        }
        AuthorTab::Art => {
            let prog = (model.author.art_progress * 100.0).round() as u32;
            let items = [
                format!("progress      {prog}%  {}", model.author.art_message),
                "g:regenerate (background)   x:cancel (keeps previous art)".into(),
                format!(
                    "previous art  {}",
                    if model.author.previous_art_intact {
                        "intact"
                    } else {
                        "replaced"
                    }
                ),
            ];
            let list_items: Vec<ListItem> = items.into_iter().map(ListItem::new).collect();
            let list = List::new(list_items)
                .block(Block::default().borders(Borders::ALL).title(" Art regen "))
                .highlight_style(Style::new().fg(Color::Black).bg(Color::LightMagenta))
                .highlight_symbol("> ");
            frame.render_stateful_widget(list, chunks[2], &mut model.author.list_state);
        }
        AuthorTab::Import => {
            let items = [
                format!("source path   {}", model.author.import_path),
                "i:import via wzt-art (Oklab + dither) into theme palette".into(),
                format!("last result   {}", model.author.import_status),
            ];
            let list_items: Vec<ListItem> = items.into_iter().map(ListItem::new).collect();
            let list = List::new(list_items)
                .block(Block::default().borders(Borders::ALL).title(" Import "))
                .highlight_style(Style::new().fg(Color::Black).bg(Color::LightMagenta))
                .highlight_symbol("> ");
            frame.render_stateful_widget(list, chunks[2], &mut model.author.list_state);
        }
    }

    let warn = if model.author.contrast_warning.is_empty() {
        "contrast: ok".into()
    } else {
        format!("contrast warning: {}", model.author.contrast_warning)
    };
    let footer = Paragraph::new(warn).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Contrast readout "),
    );
    frame.render_widget(footer, chunks[3]);

    let status = Paragraph::new(model.status.as_str())
        .block(Block::default().borders(Borders::TOP).title(" status "));
    frame.render_widget(status, chunks[4]);
}
