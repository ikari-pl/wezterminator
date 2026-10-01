//! Engine key bindings (Rust mirror of `plugin/wzt/keys.lua`) and conflict
//! detection for the keys screen.
//!
//! Binding *ids* are stable: they appear in conflict reports, doctor output and
//! this TUI. Rebinding changes `key`/`mods`; the id stays.

use std::collections::HashMap;

/// One engine binding the user can rebind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyBinding {
    pub id: &'static str,
    pub key: String,
    pub mods: String,
    pub action: &'static str,
}

/// A detected clash: same chord claimed by two bindings (or by a user chord).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyConflict {
    pub id: String,
    pub chord: String,
    pub with: String,
    pub message: String,
}

/// Built-in engine bindings. Keep in sync with `plugin/wzt/keys.lua`.
pub fn default_bindings() -> Vec<KeyBinding> {
    const ROWS: &[(&str, &str, &str, &str)] = &[
        ("palette", "P", "PRIMARY|SHIFT", "palette"),
        ("launcher", "p", "PRIMARY", "launcher"),
        ("project-picker", "O", "PRIMARY|SHIFT", "project_picker"),
        ("workspace-switcher", "S", "PRIMARY|SHIFT", "workspace_switcher"),
        ("workspace-prev", "LeftArrow", "PRIMARY|SHIFT", "workspace_prev"),
        ("workspace-next", "RightArrow", "PRIMARY|SHIFT", "workspace_next"),
        ("font-picker", "F", "CTRL|SHIFT", "pick_font"),
        ("scheme-picker", "T", "CTRL|SHIFT", "pick_scheme"),
        ("preset-undo", "Backspace", "CTRL|SHIFT", "preset_undo"),
        ("preset-next", "RightArrow", "CTRL|SHIFT", "preset_next"),
        ("preset-prev", "LeftArrow", "CTRL|SHIFT", "preset_prev"),
        ("background-pause", "G", "CTRL|SHIFT", "toggle_pause"),
        ("parallax-toggle", "B", "CTRL|SHIFT", "toggle_parallax"),
        ("parallax-recenter", "B", "CTRL|SHIFT|ALT", "recenter_parallax"),
        ("split-right", "d", "PRIMARY", "split_right"),
        ("split-down", "D", "PRIMARY|SHIFT", "split_down"),
        ("cpu-menu", "C", "PRIMARY|SHIFT", "cpu_menu"),
        ("memory-menu", "M", "PRIMARY|SHIFT", "memory_menu"),
        ("workspace-menu", "W", "PRIMARY|SHIFT", "workspace_menu"),
        ("quick-select", "u", "CTRL|SHIFT", "quick_select"),
        ("leader-split-right", "\\", "LEADER", "split_right"),
        ("leader-split-down", "-", "LEADER", "split_down"),
        ("leader-pane-left", "h", "LEADER", "pane_left"),
        ("leader-pane-down", "j", "LEADER", "pane_down"),
        ("leader-pane-up", "k", "LEADER", "pane_up"),
        ("leader-pane-right", "l", "LEADER", "pane_right"),
        ("leader-resize-left", "LeftArrow", "LEADER", "resize_left"),
        ("leader-resize-down", "DownArrow", "LEADER", "resize_down"),
        ("leader-resize-up", "UpArrow", "LEADER", "resize_up"),
        ("leader-resize-right", "RightArrow", "LEADER", "resize_right"),
        ("leader-zoom", "z", "LEADER", "pane_zoom"),
        ("leader-pane-select", "p", "LEADER", "pane_select"),
        ("leader-pane-close", "x", "LEADER", "pane_close"),
    ];
    ROWS.iter()
        .map(|(id, key, mods, action)| KeyBinding {
            id,
            key: (*key).to_string(),
            mods: (*mods).to_string(),
            action,
        })
        .collect()
}

/// Canonical chord string, matching Lua `keys.normalize` for ASCII letters.
pub fn normalize_chord(key: &str, mods: &str) -> String {
    let mut set: Vec<String> = mods
        .split(['|', ' '])
        .filter(|t| !t.is_empty())
        .map(|t| {
            let u = t.to_ascii_uppercase();
            match u.as_str() {
                "CMD" | "WIN" | "WINDOWS" => "SUPER".into(),
                "OPT" | "OPTION" => "ALT".into(),
                "CONTROL" => "CTRL".into(),
                "NONE" => String::new(),
                other => other.to_string(),
            }
        })
        .filter(|t| !t.is_empty())
        .collect();
    let key_out = if key.len() == 1 && key.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
        if !set.iter().any(|m| m == "SHIFT") {
            set.push("SHIFT".into());
        }
        key.to_ascii_lowercase()
    } else {
        key.to_ascii_lowercase()
    };
    set.sort();
    set.dedup();
    format!("{}:{}", set.join("|"), key_out)
}

/// Conflicts among `bindings`, plus optional user chords (`id` → chord).
pub fn find_conflicts(
    bindings: &[KeyBinding],
    user_chords: &[(String, String)],
) -> Vec<KeyConflict> {
    let mut by_chord: HashMap<String, Vec<String>> = HashMap::new();
    for b in bindings {
        let chord = normalize_chord(&b.key, &b.mods);
        by_chord.entry(chord).or_default().push(b.id.to_string());
    }
    let mut out = Vec::new();
    for (chord, ids) in &by_chord {
        if ids.len() > 1 {
            for id in ids {
                let others: Vec<_> = ids.iter().filter(|o| *o != id).cloned().collect();
                out.push(KeyConflict {
                    id: id.clone(),
                    chord: chord.clone(),
                    with: others.join(", "),
                    message: format!("chord {chord} also used by {}", others.join(", ")),
                });
            }
        }
    }
    for (user_id, user_chord) in user_chords {
        if let Some(ids) = by_chord.get(user_chord) {
            for id in ids {
                out.push(KeyConflict {
                    id: id.clone(),
                    chord: user_chord.clone(),
                    with: user_id.clone(),
                    message: format!(
                        "conflicts with user binding `{user_id}` on chord {user_chord}"
                    ),
                });
            }
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id).then(a.chord.cmp(&b.chord)));
    out
}

/// Whether saving should be blocked: any unresolved conflict.
pub fn save_blocked(conflicts: &[KeyConflict]) -> bool {
    !conflicts.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebind_to_in_use_chord_shows_conflict() {
        let mut bindings = default_bindings();
        // Remap palette onto the launcher chord.
        let launcher = bindings.iter().find(|b| b.id == "launcher").unwrap().clone();
        let palette = bindings.iter_mut().find(|b| b.id == "palette").unwrap();
        palette.key = launcher.key.clone();
        palette.mods = launcher.mods.clone();
        let conflicts = find_conflicts(&bindings, &[]);
        assert!(
            conflicts.iter().any(|c| c.id == "palette"),
            "expected palette conflict, got {conflicts:?}"
        );
        assert!(save_blocked(&conflicts));
    }

    #[test]
    fn user_chord_conflict() {
        let bindings = default_bindings();
        let launcher = bindings.iter().find(|b| b.id == "launcher").unwrap();
        let chord = normalize_chord(&launcher.key, &launcher.mods);
        let conflicts = find_conflicts(&bindings, &[("user:split".into(), chord)]);
        assert!(conflicts.iter().any(|c| c.id == "launcher"));
    }
}
