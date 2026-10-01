-- U10 motion: Platformer horizontal auto-scroll via call_after (not the 1 Hz
-- status tick). Coverage mirrors parallax_test.lua auto-scroll scenarios, but
-- drives the scheduler and asserts focused-window behaviour.

local T = ...
local stub_lib = T.stub

local function forget()
  for name in pairs(package.loaded) do
    if name:match('^wzt%.') and name ~= 'wzt.segments.common' and name ~= 'wzt.keys' then
      package.loaded[name] = nil
    end
  end
end

--- A 1000x500 strip like parallax_test, with horizontal parallax factors.
local function base()
  return {
    generation = 1,
    owned = {},
    background = {
      base_color = '#0a1628',
      resolution = { w = 1000, h = 500 },
      layers = {
        { source = { Color = '#0a1628' }, width = '100%', height = '100%', opacity = 1 },
        { source = { File = '/tmp/hills.png' }, opacity = 1, vertical_offset = 0, horizontal_offset = 0 },
        { source = { File = '/tmp/bricks.png' }, opacity = 1, vertical_offset = 0, horizontal_offset = 0 },
      },
      meta = {
        { vertical = 0, horizontal = 0 },
        { vertical = 0.05, horizontal = 0.35 },
        { vertical = 0, horizontal = 0.7 },
      },
    },
  }
end

--- Platformer motion defaults from themes/platformer/theme.json.
local PLATFORMER_MOTION = {
  scrollback_parallax = true,
  alt_wheel_scroll = { vertical = true, horizontal = true },
  auto_scroll = { enabled = true, speed = 8, axis = 'horizontal' },
}

local function with_motion(fn)
  local stub = stub_lib.new()
  package.loaded.wezterm = stub.wezterm
  forget()
  local clock_ms = 10000
  local parallax = require 'wzt.parallax'
  local overrides = require 'wzt.overrides'
  parallax._now_ms = function()
    return clock_ms
  end
  overrides.set_base(base())
  local env = {
    stub = stub,
    parallax = parallax,
    overrides = overrides,
    advance_ms = function(n)
      clock_ms = clock_ms + n
    end,
    window = function()
      local w = stub.new_window()
      function w:toast_notification() end
      return w
    end,
  }
  local ok, err = pcall(fn, env)
  stub.cleanup()
  forget()
  package.loaded.wezterm = nil
  if not ok then
    error(err, 0)
  end
end

---------------------------------------------------------------------------
-- call_after scheduler (Platformer)
---------------------------------------------------------------------------

T.test('Platformer: setup schedules call_after, not status-driven auto-scroll', function()
  with_motion(function(env)
    local w = env.window()
    T.eq(#env.stub.timers, 0)
    env.parallax.setup({ motion = PLATFORMER_MOTION })
    T.eq(#env.stub.timers, 1, 'one call_after queued')
    T.eq(env.stub.timers[1].after, env.parallax.TICK_S)

    -- Status tick must not advance auto-scroll while the timer owns it.
    env.stub.emit_status(w)
    T.eq(env.overrides.get_channel(w, 'autoscroll'), nil)
    T.eq(w.writes, 0)
  end)
end)

T.test('Platformer: each scheduled tick advances by speed, wraps at strip width, one write', function()
  with_motion(function(env)
    local w = env.window()
    env.parallax.setup({ motion = PLATFORMER_MOTION })
    local before = w.writes
    local offsets = {}
    -- speed 8 on a 1000px strip: wrap after 125 ticks (1000/8).
    for i = 1, 126 do
      env.stub.fire_timers()
      T.eq(w.writes, before + i, 'exactly one write per scheduled tick')
      offsets[i] = env.overrides.get_channel(w, 'autoscroll').data.horizontal
    end
    T.eq(offsets[1], 8)
    T.eq(offsets[2], 16)
    T.eq(offsets[124], 992)
    T.eq(offsets[125], 0, '1000 wraps to 0 at the strip width')
    T.eq(offsets[126], 8)
    T.eq(env.overrides.get_channel(w, 'autoscroll').data.vertical, 0)
    -- Next frame already re-scheduled.
    T.eq(#env.stub.timers, 1)
  end)
end)

T.test('Platformer: auto-scroll off schedules nothing and ticks write nothing', function()
  with_motion(function(env)
    local w = env.window()
    local off = {
      auto_scroll = { enabled = false, speed = 8, axis = 'horizontal' },
    }
    env.parallax.setup({ motion = off })
    T.eq(#env.stub.timers, 0)
    env.stub.emit_status(w)
    T.eq(env.parallax.tick(w, off), false)
    T.eq(w.writes, 0)
    T.eq(env.parallax.schedule(off), false)
    T.eq(#env.stub.timers, 0)
  end)
end)

T.test('Platformer: unfocused windows are skipped by the scheduler', function()
  with_motion(function(env)
    local focused = env.window()
    local other = env.window()
    other.focused = false
    env.parallax.setup({ motion = PLATFORMER_MOTION })
    env.stub.fire_timers()
    T.ok(env.overrides.get_channel(focused, 'autoscroll'))
    T.eq(env.overrides.get_channel(other, 'autoscroll'), nil)
    T.eq(focused.writes, 1)
    T.eq(other.writes, 0)
  end)
end)

T.test('Platformer: update-status still flushes a dirty wheel while auto-scroll runs', function()
  with_motion(function(env)
    local w = env.window()
    env.parallax.setup({ motion = PLATFORMER_MOTION })
    T.eq(env.parallax.step(w, 'horizontal', 100), true)
    local n = w.writes
    -- Throttled second step leaves dirty pending.
    T.eq(env.parallax.step(w, 'horizontal', 100), false)
    T.eq(w.writes, n)
    env.stub.emit_status(w)
    T.eq(w.writes, n + 1, 'flush_dirty wrote once')
    -- Auto channel still absent until a timer fires.
    T.eq(env.overrides.get_channel(w, 'autoscroll'), nil)
    T.eq(env.parallax.position(w).horizontal, 200)
  end)
end)
