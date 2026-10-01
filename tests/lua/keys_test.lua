-- Key bindings as data (U3): normalisation, conflict detection against the
-- user's own keys, leader and mouse bindings in add-on mode, and the shape of
-- the engine's own table.

local T = ...
local stub_lib = T.stub
local keys = require 'wzt.keys'

--- A registry that has an action for every name the engine binds.
local function full_registry()
  local reg = {}
  for _, spec in ipairs(keys.KEYS) do
    reg[spec.action] = { action = spec.action }
  end
  return reg
end

local function conflicts_of(plan, kind)
  local out = {}
  for _, c in ipairs(plan.conflicts) do
    if kind == nil or c.kind == kind then
      out[#out + 1] = c
    end
  end
  return out
end

---------------------------------------------------------------------------
-- Normalisation
---------------------------------------------------------------------------

T.test('normalize: CMD is SUPER, modifier order and case do not matter', function()
  T.eq(keys.normalize('p', 'CMD|SHIFT'), keys.normalize('p', 'shift|super'))
  T.eq(keys.normalize('p', 'ALT'), keys.normalize('p', 'OPT'))
  T.eq(keys.normalize('p', 'CTRL|ALT'), keys.normalize('p', 'ALT|CTRL'))
  T.eq(keys.normalize('p', 'NONE'), keys.normalize('p', nil))
  T.ok(keys.normalize('p', 'CMD') ~= keys.normalize('p', 'CTRL'), 'different modifiers differ')
end)

T.test('normalize: an upper-case letter means SHIFT plus the lower-case key', function()
  T.eq(keys.normalize('P', 'CMD'), keys.normalize('p', 'CMD|SHIFT'))
  T.eq(keys.normalize('P', 'CMD|SHIFT'), keys.normalize('p', 'CMD|SHIFT'))
  T.ok(keys.normalize('P', 'CMD') ~= keys.normalize('p', 'CMD'))
end)

T.test('PRIMARY expands to CMD on macOS and ALT elsewhere', function()
  T.eq(keys.expand_mods('PRIMARY|SHIFT', 'macos'), 'CMD|SHIFT')
  T.eq(keys.expand_mods('PRIMARY|SHIFT', 'linux'), 'ALT|SHIFT')
  T.eq(keys.expand_mods('CTRL|SHIFT', 'macos'), 'CTRL|SHIFT')
end)

T.test('normalize_mouse: the same trigger written two ways compares equal', function()
  local a = { event = { Down = { streak = 1, button = { WheelUp = 1 } } }, mods = 'ALT' }
  local b = { event = { Down = { streak = 1, button = { WheelUp = 1 } } }, mods = 'OPT' }
  local c = { event = { Down = { streak = 1, button = { WheelDown = 1 } } }, mods = 'ALT' }
  T.eq(keys.normalize_mouse(a), keys.normalize_mouse(b))
  T.ok(keys.normalize_mouse(a) ~= keys.normalize_mouse(c))
end)

---------------------------------------------------------------------------
-- The engine's own table
---------------------------------------------------------------------------

T.test('every binding has a unique id', function()
  local seen = {}
  for _, spec in ipairs(keys.KEYS) do
    T.ok(spec.id and spec.id ~= '', 'binding without an id')
    T.ok(not seen[spec.id], 'duplicate id ' .. spec.id)
    seen[spec.id] = true
  end
end)

T.test('the engine table does not clash with itself on any platform', function()
  for _, os_name in ipairs({ 'macos', 'linux', 'windows' }) do
    local plan = keys.plan({ os = os_name, mode = 'addon', actions = full_registry() })
    T.eq(#plan.skipped, 0, os_name .. ': every action is in the registry')
    T.eq(#plan.conflicts, 0, os_name .. ': no clash within the table')
    T.eq(#plan.keys, #keys.KEYS, os_name .. ': every binding was added')
  end
end)

T.test('a binding whose action is missing is skipped and reported, not added', function()
  local reg = full_registry()
  reg.cpu_menu = nil
  local plan = keys.plan({ os = 'macos', mode = 'addon', actions = reg })
  T.eq(#plan.skipped, 1)
  T.eq(plan.skipped[1].id, 'cpu-menu')
  T.eq(#plan.keys, #keys.KEYS - 1)
end)

---------------------------------------------------------------------------
-- Conflicts: keys
---------------------------------------------------------------------------

T.test('a disjoint user key set reports no conflicts', function()
  local plan = keys.plan({
    os = 'macos', mode = 'addon', actions = full_registry(),
    user_keys = { { key = 'F13', mods = 'CTRL' }, { key = 'q', mods = 'CMD' } },
    user_leader = { key = 'a', mods = 'CTRL' },
  })
  T.eq(#plan.conflicts, 0)
  T.eq(#plan.keys, #keys.KEYS)
end)

T.test('a user key on the same chord is a conflict, reported by id, and not overwritten', function()
  local plan = keys.plan({
    os = 'macos', mode = 'addon', actions = full_registry(),
    -- CMD|SHIFT+P, written the way a user would: SUPER and an upper-case key.
    user_keys = { { key = 'P', mods = 'SUPER|SHIFT', action = 'mine' } },
  })
  local c = conflicts_of(plan, 'key')
  T.eq(#c, 1)
  T.eq(c[1].id, 'palette')
  T.ok(c[1].message:find('already bound', 1, true))
  T.eq(#plan.keys, #keys.KEYS - 1, 'only the clashing binding is withheld')
  for _, added in ipairs(plan.added) do
    T.ok(added ~= 'palette')
  end
end)

T.test('the same chord clashes on macOS but not on Linux, where PRIMARY is a different modifier', function()
  local user = { { key = 'p', mods = 'CMD' } }
  local mac = keys.plan({ os = 'macos', mode = 'addon', actions = full_registry(), user_keys = user })
  local linux = keys.plan({ os = 'linux', mode = 'addon', actions = full_registry(), user_keys = user })
  T.eq(#mac.conflicts, 1)
  T.eq(mac.conflicts[1].id, 'launcher')
  T.eq(#linux.conflicts, 0)
end)

T.test('replace mode: the engine wins, nothing is reported', function()
  local plan = keys.plan({
    os = 'macos', mode = 'replace', actions = full_registry(),
    user_keys = { { key = 'P', mods = 'CMD|SHIFT' } },
    user_leader = { key = 'Space', mods = 'CTRL|SHIFT' },
  })
  T.eq(#plan.conflicts, 0)
  T.eq(#plan.keys, #keys.KEYS)
  T.ok(plan.leader, 'replace mode assigns the engine leader')
end)

---------------------------------------------------------------------------
-- Conflicts: leader
---------------------------------------------------------------------------

T.test('a user leader identical to the engine leader is reported', function()
  local plan = keys.plan({
    os = 'macos', mode = 'addon', actions = full_registry(),
    user_leader = { key = 'Space', mods = 'SHIFT|CTRL', timeout_milliseconds = 500 },
  })
  local c = conflicts_of(plan, 'leader')
  T.eq(#c, 1)
  T.eq(plan.leader, nil, 'the user leader is never replaced in add-on mode')
end)

T.test('a different user leader is no conflict and is left alone', function()
  local plan = keys.plan({
    os = 'macos', mode = 'addon', actions = full_registry(),
    user_leader = { key = 'a', mods = 'CTRL' },
  })
  T.eq(#conflicts_of(plan, 'leader'), 0)
  T.eq(plan.leader, nil)
end)

T.test('no user leader: the engine leader is assigned', function()
  local plan = keys.plan({ os = 'macos', mode = 'addon', actions = full_registry() })
  T.eq(plan.leader.key, 'Space')
end)

T.test('engine LEADER bindings that collide with the user LEADER keys are reported', function()
  local plan = keys.plan({
    os = 'macos', mode = 'addon', actions = full_registry(),
    user_leader = { key = 'a', mods = 'CTRL' },
    user_keys = { { key = 'h', mods = 'LEADER', action = 'mine' } },
  })
  local c = conflicts_of(plan, 'key')
  T.eq(#c, 1)
  T.eq(c[1].id, 'leader-pane-left')
end)

---------------------------------------------------------------------------
-- Conflicts: mouse
---------------------------------------------------------------------------

local function with_parallax(fn)
  local stub = stub_lib.new()
  package.loaded.wezterm = stub.wezterm
  for name in pairs(package.loaded) do
    if name:match('^wzt%.') and name ~= 'wzt.keys' and name ~= 'wzt.segments.common' then
      package.loaded[name] = nil
    end
  end
  local ok, err = pcall(fn, require 'wzt.parallax')
  stub.cleanup()
  package.loaded.wezterm = nil
  if not ok then
    error(err, 0)
  end
end

T.test('mouse: the vertical ALT+wheel bindings clash with a user ALT+wheel binding', function()
  with_parallax(function(parallax)
    local mouse = parallax.mouse_bindings({ alt_wheel_scroll = { vertical = true, horizontal = false } })
    T.eq(#mouse, 2)
    local plan = keys.plan({
      os = 'macos', mode = 'addon', actions = {},
      user_mouse = { { event = { Down = { streak = 1, button = { WheelUp = 1 } } }, mods = 'ALT', action = 'mine' } },
      mouse = mouse,
    })
    local c = conflicts_of(plan, 'mouse')
    T.eq(#c, 1)
    T.eq(c[1].id, 'parallax-vertical-up')
    T.eq(#plan.mouse, 1, 'the other direction is still added')
  end)
end)

T.test('mouse bindings follow the motion part', function()
  with_parallax(function(parallax)
    T.eq(#parallax.mouse_bindings({ alt_wheel_scroll = { vertical = false, horizontal = false } }), 0)
    T.eq(#parallax.mouse_bindings({ alt_wheel_scroll = { vertical = true, horizontal = true } }), 6)
    T.eq(#parallax.mouse_bindings(nil), 2, 'no motion part: the default is vertical only')
    for _, m in ipairs(parallax.mouse_bindings({ alt_wheel_scroll = { horizontal = true } })) do
      T.eq(m.binding.alt_screen, 'Any', 'fires in full-screen apps too')
      T.eq(m.binding.mouse_reporting, true)
    end
  end)
end)

---------------------------------------------------------------------------
-- install
---------------------------------------------------------------------------

T.test('install: the user keys stay first and untouched, engine keys follow', function()
  local mine = { key = 'F13', mods = 'CTRL', action = 'mine' }
  local config = { keys = { mine }, leader = { key = 'a', mods = 'CTRL' } }
  local plan = keys.install(config, { os = 'macos', mode = 'addon', actions = full_registry(), mouse = {} })
  T.eq(config.keys[1], mine)
  T.eq(#config.keys, 1 + #keys.KEYS)
  T.eq(config.leader.key, 'a', 'the user leader survives')
  T.eq(#plan.conflicts, 0)
end)

T.test('install: a config with no keys or leader gets the engine set', function()
  local config = {}
  keys.install(config, { os = 'linux', mode = 'addon', actions = full_registry(), mouse = {} })
  T.eq(#config.keys, #keys.KEYS)
  T.eq(config.leader.key, 'Space')
  T.eq(config.mouse_bindings, nil, 'no mouse bindings were offered')
end)

T.test('install: reading an unset field on a config_builder object does not raise', function()
  local stub = stub_lib.new({ config_keys = { keys = true, leader = true } })
  local config = stub.wezterm.config_builder()
  -- mouse_bindings is not a valid field in this stub's builder, so the write is
  -- rejected; install must still read it without error.
  T.eq(pcall(keys.install, config, { os = 'macos', mode = 'addon', actions = full_registry(), mouse = {} }), true)
  T.eq(#config.keys, #keys.KEYS)
  stub.cleanup()
end)
