//! Parsers for `wezterm ls-fonts` output.

use std::collections::BTreeSet;

/// Extract unique family names from `wezterm ls-fonts --list-system` output.
///
/// Lines look like:
/// ```text
/// wezterm.font("Fira Code", {weight="Regular", stretch="Normal", italic=false}) -- /path, FontConfig
/// ```
///
/// Family names also appear as bare strings inside `font_with_fallback` blocks
/// produced by plain `ls-fonts`; those are ignored here because `--list-system`
/// is the authoritative machine inventory.
pub fn parse_list_system(output: &str) -> Vec<String> {
    let mut families = BTreeSet::new();
    for line in output.lines() {
        if let Some(name) = family_from_wezterm_font_line(line) {
            families.insert(name);
        }
    }
    families.into_iter().collect()
}

/// Parse one `wezterm.font("Name", ...)` line into the family name.
fn family_from_wezterm_font_line(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix("wezterm.font(")?;
    let rest = rest.trim_start();
    let (name, _) = parse_lua_string(rest)?;
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// Parse a Lua single- or double-quoted string at the start of `s`.
fn parse_lua_string(s: &str) -> Option<(String, &str)> {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let quote = bytes[0];
    if quote != b'\'' && quote != b'"' {
        return None;
    }
    let mut out = String::new();
    let mut i = 1;
    while i < bytes.len() {
        let b = bytes[i];
        if b == quote {
            return Some((out, &s[i + 1..]));
        }
        if b == b'\\' {
            i += 1;
            if i >= bytes.len() {
                return None;
            }
            out.push(bytes[i] as char);
            i += 1;
            continue;
        }
        out.push(b as char);
        i += 1;
    }
    None
}

/// One glyph row from `wezterm ls-fonts --text …`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextGlyphRow {
    pub ch: String,
    pub codepoint: Option<u32>,
    pub family: Option<String>,
}

/// Parse `wezterm ls-fonts --text` output into per-glyph rows.
///
/// Example line:
/// ```text
/// a    \u{61}       x_adv=8  glyph=29   wezterm.font("Operator Mono", {weight="DemiLight", ...})
/// ```
pub fn parse_text_report(output: &str) -> Vec<TextGlyphRow> {
    let mut rows = Vec::new();
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('/') {
            continue;
        }
        // Glyph lines start with the character (possibly multi-byte) then whitespace
        // and a `\u{…}` codepoint.
        let Some(cp_idx) = trimmed.find("\\u{") else {
            continue;
        };
        let ch = trimmed[..cp_idx].trim().to_string();
        if ch.is_empty() {
            continue;
        }
        let codepoint = parse_u_brace(&trimmed[cp_idx..]);
        let family = trimmed
            .find("wezterm.font(")
            .and_then(|i| family_from_wezterm_font_line(&trimmed[i..]));
        rows.push(TextGlyphRow {
            ch,
            codepoint,
            family,
        });
    }
    rows
}

fn parse_u_brace(s: &str) -> Option<u32> {
    let rest = s.strip_prefix("\\u{")?;
    let end = rest.find('}')?;
    u32::from_str_radix(&rest[..end], 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quoted_family_with_spaces() {
        let line = r#"wezterm.font("Terminess Nerd Font Mono", {weight="Regular", stretch="Normal", italic=false}) -- /x.ttf, FontConfig"#;
        assert_eq!(
            family_from_wezterm_font_line(line).as_deref(),
            Some("Terminess Nerd Font Mono")
        );
    }
}
