-- Every key binding the engine adds, declared as DATA with ids, and the
-- conflict detection that runs against the user's own bindings.
--
-- Declaring bindings as data (rather than building `config.keys` inline as
-- metis's commands.lua did) is what makes a conflict visible: the engine can
-- compare its table with the user's, report each clash by id, and the TUI and
-- `doctor` can list the bindings without evaluating Lua.
--
-- MODIFIERS: `PRIMARY` in a binding's mods stands for the platform's main
-- command modifier: CMD on macOS, ALT elsewhere (CTRL+SHIFT is taken by
-- WezTerm's own defaults and by the appearance keys below).
--
-- ADD-ON MODE: the user's bindings win. An engine binding that collides with
-- one of theirs (same key and modifiers, after normalisation) is not added and
-- is reported as a conflict. The engine never assigns `config.leader` when the
-- user has one; a user leader identical to the engine's is reported. REPLACE
-- MODE: the engine's bindings are appended and win, because the last binding
-- for a key takes effect.

local M = {}

--- The leader the engine's LEADER bindings assume.
M.LEADER = { key = 'Space', mods = 'CTRL|SHIFT', timeout_milliseconds = 1500 }

--- The bindings. `action` names an entry of the action registry that
--- features.lua builds. Keep ids stable: they appear in reports and docs.
M.KEYS = {
  -- Palette and launchers.
  { id = 'palette', key = 'P', mods = 'PRIMARY|SHIFT', action = 'palette' },
  { id = 'launcher', key = 'p', mods = 'PRIMARY', action = 'launcher' },
  { id = 'project-picker', key = 'O', mods = 'PRIMARY|SHIFT', action = 'project_picker' },
  { id = 'workspace-switcher', key = 'S', mods = 'PRIMARY|SHIFT', action = 'workspace_switcher' },
  { id = 'workspace-prev', key = 'LeftArrow', mods = 'PRIMARY|SHIFT', action = 'workspace_prev' },
  { id = 'workspace-next', key = 'RightArrow', mods = 'PRIMARY|SHIFT', action = 'workspace_next' },

  -- Appearance.
  { id = 'font-picker', key = 'F', mods = 'CTRL|SHIFT', action = 'pick_font' },
  { id = 'scheme-picker', key = 'T', mods = 'CTRL|SHIFT', action = 'pick_scheme' },
  { id = 'preset-undo', key = 'Backspace', mods = 'CTRL|SHIFT', action = 'preset_undo' },
  { id = 'preset-next', key = 'RightArrow', mods = 'CTRL|SHIFT', action = 'preset_next' },
  { id = 'preset-prev', key = 'LeftArrow', mods = 'CTRL|SHIFT', action = 'preset_prev' },
  { id = 'background-pause', key = 'G', mods = 'CTRL|SHIFT', action = 'toggle_pause' },
  { id = 'parallax-toggle', key = 'B', mods = 'CTRL|SHIFT', action = 'toggle_parallax' },
  { id = 'parallax-recenter', key = 'B', mods = 'CTRL|SHIFT|ALT', action = 'recenter_parallax' },

  -- Splits, matching iTerm2's KEYS rather than its vocabulary: SplitHorizontal
  -- puts the new pane to the right.
  { id = 'split-right', key = 'd', mods = 'PRIMARY', action = 'split_right' },
  { id = 'split-down', key = 'D', mods = 'PRIMARY|SHIFT', action = 'split_down' },

  -- Status-bar segments cannot be clicked (WezTerm gives Lua no pointer
  -- position), so each gets a key that opens the equivalent menu.
  { id = 'cpu-menu', key = 'C', mods = 'PRIMARY|SHIFT', action = 'cpu_menu' },
  { id = 'memory-menu', key = 'M', mods = 'PRIMARY|SHIFT', action = 'memory_menu' },
  { id = 'workspace-menu', key = 'W', mods = 'PRIMARY|SHIFT', action = 'workspace_menu' },

  { id = 'quick-select', key = 'u', mods = 'CTRL|SHIFT', action = 'quick_select' },

  -- Panes, under the leader.
  { id = 'leader-split-right', key = '\\', mods = 'LEADER', action = 'split_right' },
  { id = 'leader-split-down', key = '-', mods = 'LEADER', action = 'split_down' },
  { id = 'leader-pane-left', key = 'h', mods = 'LEADER', action = 'pane_left' },
  { id = 'leader-pane-down', key = 'j', mods = 'LEADER', action = 'pane_down' },
  { id = 'leader-pane-up', key = 'k', mods = 'LEADER', action = 'pane_up' },
  { id = 'leader-pane-right', key = 'l', mods = 'LEADER', action = 'pane_right' },
  { id = 'leader-resize-left', key = 'LeftArrow', mods = 'LEADER', action = 'resize_left' },
  { id = 'leader-resize-down', key = 'DownArrow', mods = 'LEADER', action = 'resize_down' },
  { id = 'leader-resize-up', key = 'UpArrow', mods = 'LEADER', action = 'resize_up' },
  { id = 'leader-resize-right', key = 'RightArrow', mods = 'LEADER', action = 'resize_right' },
  { id = 'leader-zoom', key = 'z', mods = 'LEADER', action = 'pane_zoom' },
  { id = 'leader-pane-select', key = 'p', mods = 'LEADER', action = 'pane_select' },
  { id = 'leader-pane-close', key = 'x', mods = 'LEADER', action = 'pane_close' },
}

---------------------------------------------------------------------------
-- Normalisation
---------------------------------------------------------------------------

local MOD_ALIAS = {
  SUPER = 'SUPER', CMD = 'SUPER', WIN = 'SUPER', WINDOWS = 'SUPER',
  ALT = 'ALT', OPT = 'ALT', OPTION = 'ALT',
  CTRL = 'CTRL', CONTROL = 'CTRL',
  SHIFT = 'SHIFT',
  LEADER = 'LEADER',
  NONE = false,
}

--- Replace the PRIMARY token for `os_name` ('macos' or anything else).
function M.expand_mods(mods, os_name)
  if type(mods) ~= 'string' then
    return mods
  end
  local primary = os_name == 'macos' and 'CMD' or 'ALT'
  return (mods:gsub('PRIMARY', primary))
end

local function mod_set(mods)
  local set = {}
  if type(mods) == 'string' then
    for token in mods:gmatch('[^|%s]+') do
      local canon = MOD_ALIAS[token:upper()]
      if canon then
        set[canon] = true
      elseif canon == nil then
        set[token:upper()] = true -- unknown token: keep it, it still distinguishes
      end
    end
  end
  return set
end

local function mods_string(set)
  local list = {}
  for m in pairs(set) do
    list[#list + 1] = m
  end
  table.sort(list)
  return table.concat(list, '|')
end

--- Canonical form of a modifier string: aliases folded (CMD is SUPER, OPT is
--- ALT), sorted, NONE dropped.
function M.normalize_mods(mods)
  return mods_string(mod_set(mods))
end

--- Canonical form of a key chord: "ALT|SHIFT:p". A single upper-case letter
--- means SHIFT plus the lower-case key, as WezTerm treats it.
function M.normalize(key, mods)
  local set = mod_set(mods)
  key = tostring(key or '')
  if key:match('^%u$') then
    set.SHIFT = true
  end
  return mods_string(set) .. ':' .. key:lower()
end

local function button_name(button)
  if type(button) == 'string' then
    return button:lower()
  elseif type(button) == 'table' then
    for name in pairs(button) do
      return tostring(name):lower()
    end
  end
  return '?'
end

--- Canonical form of a mouse binding's trigger: "ALT:down:1:wheelup".
function M.normalize_mouse(binding)
  local kind, streak, button = '?', 1, '?'
  for k, v in pairs(binding.event or {}) do
    kind = tostring(k):lower()
    if type(v) == 'table' then
      streak = v.streak or 1
      button = button_name(v.button)
    else
      button = button_name(v)
    end
  end
  return string.format('%s:%s:%s:%s', M.normalize_mods(binding.mods), kind, streak, button)
end

---------------------------------------------------------------------------
-- Planning
---------------------------------------------------------------------------

local function describe(key, mods)
  return tostring(mods or '') ~= '' and (tostring(mods) .. '+' .. tostring(key)) or tostring(key)
end

--- Work out what to add and what clashes. Pure: touches no config.
---
--- opts = {
---   os            'macos' | 'linux' | 'windows'
---   mode          'addon' | 'replace'
---   user_keys     config.keys as the user left it, or nil
---   user_leader   config.leader as the user left it, or nil
---   user_mouse    config.mouse_bindings as the user left it, or nil
---   mouse         { {id =, binding =}, ... } engine mouse bindings (parallax.mouse_bindings)
---   actions       registry name -> action; bindings whose action is missing are skipped
---   keys          binding table, default M.KEYS (tests pass their own)
--- }
---
--- Returns {
---   keys       = engine bindings to append, as WezTerm tables
---   mouse      = engine mouse bindings to append
---   leader     = the leader to assign, or nil
---   added      = { ids }
---   conflicts  = { {kind = 'key'|'mouse'|'leader', id =, ...}, ... }
---   skipped    = { {id =, reason =}, ... }
--- }
function M.plan(opts)
  local addon = opts.mode ~= 'replace'
  local os_name = opts.os or 'linux'
  local actions = opts.actions or {}
  local result = { keys = {}, mouse = {}, added = {}, conflicts = {}, skipped = {} }

  -- Leader.
  local ul = opts.user_leader
  if ul then
    if M.normalize(ul.key, ul.mods) == M.normalize(M.LEADER.key, M.LEADER.mods) then
      result.conflicts[#result.conflicts + 1] = {
        kind = 'leader',
        id = 'leader',
        key = M.LEADER.key,
        mods = M.LEADER.mods,
        message = 'your leader is the same chord as the engine leader ('
          .. describe(M.LEADER.key, M.LEADER.mods) .. '); engine LEADER bindings share it with yours',
      }
    end
    if not addon then
      result.leader = M.LEADER
    end
  else
    result.leader = M.LEADER
  end

  -- Keys.
  local taken = {}
  for _, k in ipairs(opts.user_keys or {}) do
    taken[M.normalize(k.key, k.mods)] = k
  end
  for _, spec in ipairs(opts.keys or M.KEYS) do
    local mods = M.expand_mods(spec.mods, os_name)
    local action = actions[spec.action]
    if action == nil then
      result.skipped[#result.skipped + 1] = { id = spec.id, reason = 'no action "' .. tostring(spec.action) .. '"' }
    else
      local sig = M.normalize(spec.key, mods)
      local clash = taken[sig]
      if clash and addon then
        result.conflicts[#result.conflicts + 1] = {
          kind = 'key',
          id = spec.id,
          key = spec.key,
          mods = mods,
          message = describe(spec.key, mods) .. ' is already bound in your config',
        }
      else
        result.keys[#result.keys + 1] = { key = spec.key, mods = mods, action = action }
        result.added[#result.added + 1] = spec.id
        taken[sig] = spec -- the engine's own table must not clash with itself either
      end
    end
  end

  -- Mouse.
  local mtaken = {}
  for _, b in ipairs(opts.user_mouse or {}) do
    mtaken[M.normalize_mouse(b)] = b
  end
  for _, entry in ipairs(opts.mouse or {}) do
    local sig = M.normalize_mouse(entry.binding)
    if mtaken[sig] and addon then
      result.conflicts[#result.conflicts + 1] = {
        kind = 'mouse',
        id = entry.id,
        mods = entry.binding.mods,
        message = 'mouse trigger ' .. sig .. ' is already bound in your config',
      }
    else
      result.mouse[#result.mouse + 1] = entry.binding
      result.added[#result.added + 1] = entry.id
      mtaken[sig] = entry
    end
  end

  return result
end

--- Plan against `config` as it stands and assign the result. The user's own
--- entries stay first and untouched. Returns the plan.
function M.install(config, opts)
  -- config_builder() objects may complain about reads of unset fields.
  local function read(name)
    local ok, v = pcall(function()
      return config[name]
    end)
    return ok and type(v) == 'table' and v or nil
  end
  local user_keys, user_mouse, user_leader = read('keys'), read('mouse_bindings'), read('leader')

  local plan = M.plan({
    os = opts.os,
    mode = opts.mode,
    user_keys = user_keys,
    user_leader = user_leader,
    user_mouse = user_mouse,
    mouse = opts.mouse,
    actions = opts.actions,
    keys = opts.keys,
  })

  if #plan.keys > 0 then
    local list = {}
    for _, k in ipairs(user_keys or {}) do
      list[#list + 1] = k
    end
    for _, k in ipairs(plan.keys) do
      list[#list + 1] = k
    end
    config.keys = list
  end
  if #plan.mouse > 0 then
    local list = {}
    for _, b in ipairs(user_mouse or {}) do
      list[#list + 1] = b
    end
    for _, b in ipairs(plan.mouse) do
      list[#list + 1] = b
    end
    config.mouse_bindings = list
  end
  if plan.leader then
    config.leader = plan.leader
  end
  return plan
end

return M
