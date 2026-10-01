-- wezterminator: composable WezTerm presets.
--
-- Add-on users load this as a plugin:
--
--   local wzt = wezterm.plugin.require 'https://github.com/<owner>/wezterminator'
--   wzt.apply_to_config(config)
--
-- Replace mode loads the same entry point from a local checkout:
--
--   local wzt = dofile(checkout .. '/plugin/init.lua')
--   wzt.apply_to_config(config, { dir = checkout })
--
-- One code path serves both. Nothing below touches the disk during config
-- evaluation: files are written only from event handlers and actions.

local wezterm = require 'wezterm'

local M = {}

M.VERSION = '0.1.0'

--- Highest document schema_version this engine reads.
M.SCHEMA_VERSION = 1

--- Used when the active preset is missing or unset.
M.DEFAULT_PRESET = 'builtin:cpc-cool'

-- Filled by apply_to_config. Actions are created there because
-- wezterm.action_callback needs the live wezterm module.
M.actions = {}

---------------------------------------------------------------------------
-- Locating the plugin
---------------------------------------------------------------------------

-- The directory two levels above this file: <root>/plugin/init.lua -> <root>.
local function own_dir()
  local ok, info = pcall(debug.getinfo, 1, 'S')
  local src = ok and info and info.source
  if type(src) == 'string' and src:sub(1, 1) == '@' then
    local root = src:sub(2):match('^(.*)[/\\]plugin[/\\]init%.lua$')
    if root and root ~= '' then
      return root
    end
  end
  return nil
end

local function file_exists(path)
  local f = io.open(path, 'rb')
  if f then
    f:close()
    return true
  end
  return false
end

local function find_plugin_dir(opts)
  if opts.dir then
    return opts.dir
  end
  local own = own_dir()
  if own then
    return own
  end
  -- wezterm.plugin.list() describes plugins loaded with plugin.require.
  local pattern = opts.plugin_pattern or 'wezterminator'
  local ok, list = pcall(wezterm.plugin.list)
  if ok and type(list) == 'table' then
    for _, entry in ipairs(list) do
      local dir = entry.plugin_dir
      if dir and (tostring(entry.url or ''):find(pattern, 1, true) or tostring(dir):find(pattern, 1, true))
        and file_exists(dir .. '/plugin/init.lua') then
        return dir
      end
    end
  end
  return nil
end

---------------------------------------------------------------------------
-- Add-on mode: what did the user's config already set?
---------------------------------------------------------------------------

-- config_builder() stores assigned keys with raw_set, so pairs() should see
-- them. If a WezTerm build does not, `opts.keep` lists the keys by hand.
local function snapshot_keys(config, keep)
  local seen, keys = {}, {}
  pcall(function()
    for k, v in pairs(config) do
      if type(k) == 'string' and v ~= nil and not seen[k] then
        seen[k] = true
        keys[#keys + 1] = k
      end
    end
  end)
  for _, k in ipairs(keep or {}) do
    if type(k) == 'string' and not seen[k] then
      seen[k] = true
      keys[#keys + 1] = k
    end
  end
  table.sort(keys)
  return keys
end

local MODES = {
  addon = 'addon', ['add-on'] = 'addon',
  replace = 'replace', ['replace-import'] = 'replace',
}

---------------------------------------------------------------------------
-- apply_to_config
---------------------------------------------------------------------------

--- Resolve the active preset and apply it to `config`.
---
--- opts (all optional):
---   dir            plugin checkout directory (otherwise found automatically)
---   mode           'addon' or 'replace'; default: state.json's install_mode, else 'addon'
---   keep           extra config keys to treat as user-owned in add-on mode
---   dirs           { ['local'] =, fleet =, state =, data = } overriding platform.dirs()
---   installed_fonts  list of installed family names; default unknown (nil)
---   default_preset id used when the active preset is missing
---
--- Returns a summary table (also kept in M.last) for tests and diagnostics.
function M.apply_to_config(config, opts)
  opts = opts or {}

  local dir = find_plugin_dir(opts)
  if not dir then
    wezterm.log_error('wezterminator: cannot find its own directory; pass { dir = <checkout> }')
    return nil
  end
  local plugin_path = dir .. '/plugin/?.lua'
  if not package.path:find(plugin_path, 1, true) then
    package.path = plugin_path .. ';' .. package.path
  end

  local platform = require 'wzt.platform'
  local data = require 'wzt.data'
  local resolve = require 'wzt.resolve'
  local apply = require 'wzt.apply'
  local overrides = require 'wzt.overrides'
  local state = require 'wzt.state'

  -- Snapshot BEFORE anything below can assign to `config`.
  local snapshot = snapshot_keys(config, opts.keep)

  local dirs = platform.dirs(opts)
  local where = { builtin = dir, dirs = dirs }
  local host = platform.info()

  local loaded = data.load(where)

  local mode = MODES[opts.mode or '']
    or MODES[(loaded.state and loaded.state.install_mode) or '']
    or 'addon'
  local owned = (mode == 'addon') and snapshot or {}
  local owned_set = {}
  for _, k in ipairs(owned) do
    owned_set[k] = true
  end

  local result = resolve.resolve({
    engine = {
      supported_schema_version = M.SCHEMA_VERSION,
      default_preset = opts.default_preset or M.DEFAULT_PRESET,
    },
    layers = loaded.layers,
    state = loaded.state,
    environment = { installed_fonts = opts.installed_fonts },
    addon = (mode == 'addon') and { owned_keys = owned } or nil,
  })

  local summary = {
    plugin_dir = dir,
    dirs = dirs,
    host = host,
    mode = mode,
    owned = owned,
    generation = loaded.generation,
    generation_changed = loaded.changed,
    result = result,
    load_errors = loaded.errors,
    unavailable = {},
  }

  if result.resolved then
    local fragment, unavailable = apply.fragment(result.resolved, host)
    summary.unavailable = unavailable

    -- Never assign a key the user's config owns (add-on precedence).
    for k, v in pairs(fragment) do
      if not owned_set[k] then
        config[k] = v
      end
    end

    -- The background stack is data, kept for the aggregator; it is also what
    -- the config itself starts with.
    local art = result.resolved.parts.art
    local bg
    if art then
      local recorded = state.read_screens(dirs)
      bg = apply.background(art, {
        where = where,
        machine = result.machine,
        resolution = apply.pick_resolution(recorded, result.machine.screen_overrides),
        default_base = result.resolved.parts.palette
          and result.resolved.parts.palette.ui
          and result.resolved.parts.palette.ui.bg,
      })
      if not owned_set.background then
        config.background = apply.rest_layers(bg)
      end
    end
    summary.background = bg
    overrides.set_base({
      generation = loaded.generation,
      background = bg,
      owned = owned,
      notices = result.notices,
      unavailable = unavailable,
      active_id = result.active.id,
      mode = mode,
    })
  else
    overrides.set_base({
      generation = loaded.generation,
      owned = owned,
      notices = result.notices,
      mode = mode,
    })
  end

  -- Reload on change: state.json and the local/fleet layer files. NOT
  -- screens.json or engine.json, and not the state directory itself.
  for _, path in ipairs(data.watch_paths(dirs)) do
    wezterm.add_to_config_reload_watch_list(path)
  end

  overrides.setup()

  -- Everything that writes a file runs from a GUI event, once per process, and
  -- only when the content differs. wezterm.gui is nil inside the mux server.
  wezterm.on('update-status', function(_window, _pane)
    local proc = platform.store_get('wzt_process') or {}
    if proc.recorded or not wezterm.gui then
      return
    end
    proc.recorded = true
    platform.store_set('wzt_process', proc)
    pcall(state.record_environment, dirs, {
      plugin_dir = dir,
      version = M.VERSION,
      schema_version = M.SCHEMA_VERSION,
    })
  end)

  -- Presets that cycling visits: everything not shadowed by a higher layer.
  local cycle_ids = {}
  for _, entry in ipairs(result.catalog) do
    if entry.shadowed_by == nil then
      cycle_ids[#cycle_ids + 1] = entry.id
    end
  end
  local active_id = result.active.id

  local function toast(window, text)
    pcall(function()
      window:toast_notification('wezterminator', text, nil, 2500)
    end)
  end

  local function commit(window, id)
    local ok, err = state.commit(dirs, id)
    if not ok then
      toast(window, 'Cannot switch preset: ' .. tostring(err))
    end
    return ok
  end

  local function cycle(window, step)
    local id, err = state.cycle(dirs, step, cycle_ids, active_id)
    if id then
      toast(window, 'Preset: ' .. id)
    else
      toast(window, 'Cannot switch preset: ' .. tostring(err))
    end
  end

  local function undo(window)
    local id, err = state.undo(dirs)
    toast(window, id and ('Preset: ' .. id) or ('Nothing to undo (' .. tostring(err) .. ')'))
  end

  wezterm.on('wzt.commit-preset', function(window, _pane, id)
    commit(window, id)
  end)
  wezterm.on('wzt.cycle-next', function(window)
    cycle(window, 1)
  end)
  wezterm.on('wzt.cycle-prev', function(window)
    cycle(window, -1)
  end)
  wezterm.on('wzt.undo', function(window)
    undo(window)
  end)

  if wezterm.action_callback then
    M.actions.cycle_next = wezterm.action_callback(function(window)
      cycle(window, 1)
    end)
    M.actions.cycle_prev = wezterm.action_callback(function(window)
      cycle(window, -1)
    end)
    M.actions.undo = wezterm.action_callback(function(window)
      undo(window)
    end)
    --- A callback action that commits `id`, for palette entries and key bindings.
    M.actions.commit = function(id)
      return wezterm.action_callback(function(window)
        commit(window, id)
      end)
    end
  end

  M.last = summary
  return summary
end

return M
