-- The command palette. Entries are declared as data naming actions in the
-- registry that features.lua builds, so the palette and the key bindings
-- (keys.lua) share one set of actions, and a missing action (say, a platform
-- without ps) drops its entry rather than breaking the whole palette.
--
-- Open it with the `palette` binding; the launcher binding opens WezTerm's
-- fuzzy launcher (tabs, workspaces, key assignments, domains) instead.
--
-- Metis's palette emitted events that feature modules handled. Here each
-- feature exposes callback actions directly, so there is no event name to keep
-- in step.

local wezterm = require 'wezterm'

local M = {}

--- { brief, action = <registry name> }. The `brief` prefix groups entries in
--- the palette's own sort order.
M.ENTRIES = {
  { brief = 'Appearance: Choose color scheme', action = 'pick_scheme' },
  { brief = 'Appearance: Choose font', action = 'pick_font' },
  { brief = 'Appearance: Next preset', action = 'preset_next' },
  { brief = 'Appearance: Previous preset', action = 'preset_prev' },
  { brief = 'Appearance: Undo last preset change', action = 'preset_undo' },
  { brief = 'Appearance: Toggle background layers', action = 'toggle_pause' },
  { brief = 'Appearance: Toggle virtual parallax (ALT+wheel)', action = 'toggle_parallax' },
  { brief = 'Appearance: Recentre parallax', action = 'recenter_parallax' },
  { brief = 'Appearance: Increase font size', action = 'font_bigger' },
  { brief = 'Appearance: Decrease font size', action = 'font_smaller' },
  { brief = 'Appearance: Reset font size', action = 'font_reset' },

  { brief = 'Navigate: Open project workspace', action = 'project_picker' },
  { brief = 'Navigate: Fuzzy tabs, workspaces, commands, and domains', action = 'launcher' },
  { brief = 'Navigate: Open URL, issue, or git commit', action = 'quick_select' },
  { brief = 'Navigate: Search scrollback', action = 'search' },
  { brief = 'Navigate: Enter copy mode', action = 'copy_mode' },
  { brief = 'Navigate: Pick pane', action = 'pane_select' },

  { brief = 'System: Top processes by CPU', action = 'cpu_menu' },
  { brief = 'System: Top processes by memory', action = 'memory_menu' },
  { brief = 'Workspace: Manage workspaces', action = 'workspace_menu' },
  { brief = 'Workspace: Switch existing workspace', action = 'workspace_switcher' },
  { brief = 'Workspace: Rename current workspace', action = 'workspace_rename' },

  { brief = 'Pane: Split right', action = 'split_right' },
  { brief = 'Pane: Split down', action = 'split_down' },
  { brief = 'Pane: Toggle zoom', action = 'pane_zoom' },
  { brief = 'Pane: Rotate clockwise', action = 'pane_rotate_cw' },
  { brief = 'Pane: Rotate counter-clockwise', action = 'pane_rotate_ccw' },
  { brief = 'Pane: Close', action = 'pane_close' },

  { brief = 'Tab: New at current directory', action = 'tab_at_cwd' },
  { brief = 'Tab: Rename', action = 'tab_rename' },
  { brief = 'Tab: Move left', action = 'tab_move_left' },
  { brief = 'Tab: Move right', action = 'tab_move_right' },
  { brief = 'Tab: Close', action = 'tab_close' },

  { brief = 'Project: Open current directory', action = 'open_cwd' },
  { brief = 'Project: Open current GitHub repository', action = 'open_github' },

  { brief = 'Config: Edit overrides', action = 'edit_overrides' },
  { brief = 'Config: Edit machine settings', action = 'edit_machine' },
  { brief = 'Config: Reload WezTerm', action = 'reload' },
  { brief = 'Config: Show debug overlay', action = 'debug_overlay' },
}

--- Resolve the entries against `registry`. `presets` is the list of
--- { id =, name = } the user can switch to; `commit(id)` builds the action
--- that switches. Returns the list for `augment-command-palette` and the
--- names of entries that were left out.
function M.build(registry, presets, commit)
  local out, missing = {}, {}
  for _, e in ipairs(M.ENTRIES) do
    local action = registry[e.action]
    if action ~= nil then
      out[#out + 1] = { brief = e.brief, action = action }
    else
      missing[#missing + 1] = e.action
    end
  end
  if commit then
    for _, p in ipairs(presets or {}) do
      out[#out + 1] = { brief = 'Preset: ' .. (p.name or p.id), action = commit(p.id) }
    end
  end
  return out, missing
end

--- Register `augment-command-palette`.
function M.setup(entries)
  wezterm.on('augment-command-palette', function()
    return entries
  end)
end

return M
