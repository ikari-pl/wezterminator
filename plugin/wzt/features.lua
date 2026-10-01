-- Wires the ported metis features onto the engine: status bar, parallax,
-- background pause, projects, quick-select, inspect menus, pickers, key
-- bindings and the command palette. init.lua calls `install` once per
-- evaluation, after the preset has been applied and the engine's own actions
-- (cycle, undo, commit) exist.
--
-- It owns the ACTION REGISTRY: a name -> WezTerm action table that keys.lua
-- binds and palette.lua lists, so a feature is declared once and reachable
-- both ways.
--
-- opts.features switches parts off for users who want only presets:
--   false                      nothing below is installed
--   { keys = false, ... }      that part is skipped
-- parts: keys, status, parallax, palette.

local wezterm = require 'wezterm'
local inspect = require 'wzt.inspect'
local keys = require 'wzt.keys'
local palette = require 'wzt.palette'
local parallax = require 'wzt.parallax'
local pickers = require 'wzt.pickers'
local projects = require 'wzt.projects'
local quickselect = require 'wzt.quickselect'
local status = require 'wzt.status'

local M = {}

local function enabled(features, name)
  if features == false then
    return false
  end
  if type(features) == 'table' and features[name] == false then
    return false
  end
  return true
end

--- Actions that need only WezTerm itself.
function M.basic_actions()
  local act = wezterm.action
  local function prompt(description, apply)
    return act.PromptInputLine({
      description = description,
      action = wezterm.action_callback(function(window, _, line)
        if line and line ~= '' then
          apply(window, line)
        end
      end),
    })
  end
  return {
    palette = act.ActivateCommandPalette,
    launcher = act.ShowLauncherArgs({
      title = 'Navigate WezTerm',
      flags = 'FUZZY|TABS|WORKSPACES|COMMANDS|KEY_ASSIGNMENTS|LAUNCH_MENU_ITEMS|DOMAINS',
    }),
    workspace_switcher = act.ShowLauncherArgs({ title = 'Switch workspace', flags = 'FUZZY|WORKSPACES' }),
    workspace_prev = act.SwitchWorkspaceRelative(-1),
    workspace_next = act.SwitchWorkspaceRelative(1),

    split_right = act.SplitHorizontal({ domain = 'CurrentPaneDomain' }),
    split_down = act.SplitVertical({ domain = 'CurrentPaneDomain' }),
    pane_left = act.ActivatePaneDirection('Left'),
    pane_down = act.ActivatePaneDirection('Down'),
    pane_up = act.ActivatePaneDirection('Up'),
    pane_right = act.ActivatePaneDirection('Right'),
    resize_left = act.AdjustPaneSize({ 'Left', 3 }),
    resize_down = act.AdjustPaneSize({ 'Down', 3 }),
    resize_up = act.AdjustPaneSize({ 'Up', 3 }),
    resize_right = act.AdjustPaneSize({ 'Right', 3 }),
    pane_zoom = act.TogglePaneZoomState,
    pane_select = act.PaneSelect({ alphabet = '1234567890' }),
    pane_close = act.CloseCurrentPane({ confirm = true }),
    pane_rotate_cw = act.RotatePanes('Clockwise'),
    pane_rotate_ccw = act.RotatePanes('CounterClockwise'),

    tab_rename = prompt('New tab title', function(window, line)
      window:active_tab():set_title(line)
    end),
    tab_move_left = act.MoveTabRelative(-1),
    tab_move_right = act.MoveTabRelative(1),
    tab_close = act.CloseCurrentTab({ confirm = true }),

    search = act.Search('CurrentSelectionOrEmptyString'),
    copy_mode = act.ActivateCopyMode,
    font_bigger = act.IncreaseFontSize,
    font_smaller = act.DecreaseFontSize,
    font_reset = act.ResetFontSize,
    reload = act.ReloadConfiguration,
    debug_overlay = act.ShowDebugOverlay,
  }
end

--- Build the registry. `ctx.engine_actions` is init.lua's M.actions
--- (cycle_next, cycle_prev, undo, commit), absent where wezterm.action_callback
--- is.
function M.registry(ctx)
  local reg = M.basic_actions()
  local act = wezterm.action
  -- Needs the mux, so it is built here rather than with the pure-WezTerm set.
  reg.workspace_rename = act.PromptInputLine({
    description = 'New workspace name',
    action = wezterm.action_callback(function(_, _, line)
      if line and line ~= '' then
        wezterm.mux.rename_workspace(wezterm.mux.get_active_workspace(), line)
      end
    end),
  })

  local function merge(t)
    for k, v in pairs(t) do
      reg[k] = v
    end
  end
  merge(parallax.actions())
  merge(inspect.actions())
  merge(quickselect.actions({
    machine = ctx.machine,
    edit = {
      overrides = ctx.dirs['local'] .. '/overrides.json',
      machine = ctx.dirs['local'] .. '/machine.json',
    },
  }))
  merge(pickers.actions({
    resolved = ctx.resolved,
    host = ctx.host,
    dirs = ctx.dirs,
    fonts = ctx.fonts,
  }))
  reg.project_picker = projects.action(ctx.machine)

  local engine = ctx.engine_actions or {}
  reg.preset_next = engine.cycle_next
  reg.preset_prev = engine.cycle_prev
  reg.preset_undo = engine.undo
  return reg
end

--- Install everything. ctx = {
---   mode, owned_set, resolved (may be nil), machine, dirs, host, catalog,
---   engine_actions, features (opts.features), binary, collect, fonts
--- }
--- Returns a summary: { keys = {added, conflicts, skipped}, status, palette_entries, ... }
function M.install(config, ctx)
  local summary = {}
  if ctx.features == false then
    return summary
  end
  ctx.machine = ctx.machine or {}
  local resolved = ctx.resolved
  local motion = resolved and resolved.parts.motion
  local registry = M.registry(ctx)

  if enabled(ctx.features, 'keys') then
    local plan = keys.install(config, {
      os = ctx.host and ctx.host.os,
      mode = ctx.mode,
      actions = registry,
      mouse = parallax.mouse_bindings(motion),
    })
    summary.keys = { added = plan.added, conflicts = plan.conflicts, skipped = plan.skipped }
    for _, c in ipairs(plan.conflicts) do
      wezterm.log_warn('wezterminator: key conflict (' .. c.id .. '): ' .. c.message)
    end
  end

  if enabled(ctx.features, 'status') then
    summary.status = status.setup(config, {
      resolved = resolved,
      dirs = ctx.dirs,
      owned_set = ctx.owned_set,
      binary = ctx.binary,
      collect = ctx.collect,
    })
  end

  if enabled(ctx.features, 'parallax') and resolved then
    parallax.setup({ motion = motion })
  end

  if enabled(ctx.features, 'palette') then
    local presets = {}
    for _, entry in ipairs(ctx.catalog or {}) do
      if entry.shadowed_by == nil then
        presets[#presets + 1] = { id = entry.id, name = entry.name }
      end
    end
    local engine = ctx.engine_actions or {}
    local entries, missing = palette.build(registry, presets, engine.commit)
    palette.setup(entries)
    summary.palette_entries = #entries
    summary.palette_missing = missing
  end

  return summary
end

return M
