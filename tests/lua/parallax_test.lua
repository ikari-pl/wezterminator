-- Parallax, auto-scroll and background pause (U3). They are channels of the
-- override aggregator; these tests count aggregator writes, the quantity the
-- plan puts a number on:
--   * pause then restore: one write per toggle, parallax offsets intact (R18)
--   * auto-scroll with wrapping: exactly one write per tick
--   * auto-scroll off: ticks write nothing (R16)

local T = ...
local stub_lib = T.stub

local function forget()
  for name in pairs(package.loaded) do
    if name:match('^wzt%.') and name ~= 'wzt.segments.common' and name ~= 'wzt.keys' then
      package.loaded[name] = nil
    end
  end
end

--- A base whose layers have parallax factors, built for a 1000x500 art strip.
local function base()
  return {
    generation = 1,
    owned = {},
    background = {
      base_color = '#0c0c18',
      resolution = { w = 1000, h = 500 },
      layers = {
        { source = { Color = '#0c0c18' }, width = '100%', height = '100%', opacity = 1 },
        { source = { File = '/tmp/stars.png' }, opacity = 1, vertical_offset = 0, horizontal_offset = 0 },
        { source = { File = '/tmp/grid.png' }, opacity = 1, vertical_offset = 0, horizontal_offset = 0 },
      },
      meta = {
        { vertical = 0, horizontal = 0 },
        { vertical = 0.1, horizontal = 0.1 },
        { vertical = 0.5, horizontal = 0.5 },
      },
    },
  }
end

local function with_parallax(fn)
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

local function bg_of(window)
  local o = window:get_config_overrides()
  return o and o.background
end

---------------------------------------------------------------------------
-- Wheel
---------------------------------------------------------------------------

T.test('wheel: a step writes the parallax channel, layers move by their own factor', function()
  with_parallax(function(env)
    local w = env.window()
    T.eq(env.parallax.step(w, 'vertical', 100), true)
    local bg = bg_of(w)
    T.ok(bg, 'background override written')
    T.eq(bg[1].vertical_offset, nil, 'the base colour never moves')
    T.eq(bg[2].vertical_offset, 10, 'factor 0.1 of 100')
    T.eq(bg[3].vertical_offset, 50, 'factor 0.5 of 100')
    T.eq(env.parallax.position(w).vertical, 100)
  end)
end)

T.test('wheel: vertical and horizontal are independent channels of one position', function()
  with_parallax(function(env)
    local w = env.window()
    env.parallax.step(w, 'vertical', 100)
    env.advance_ms(100)
    env.parallax.step(w, 'horizontal', -60)
    local bg = bg_of(w)
    T.eq(bg[3].vertical_offset, 50)
    T.eq(bg[3].horizontal_offset, -30)
  end)
end)

T.test('wheel: the virtual position is bounded', function()
  with_parallax(function(env)
    local w = env.window()
    for _ = 1, 50 do
      env.advance_ms(100)
      env.parallax.step(w, 'vertical', 90)
    end
    T.eq(env.parallax.position(w).vertical, env.parallax.RANGE)
  end)
end)

T.test('wheel: events faster than the throttle cost no write; the next tick flushes them in one', function()
  with_parallax(function(env)
    local w = env.window()
    T.eq(env.parallax.step(w, 'vertical', 90), true)
    local n = w.writes
    T.eq(env.parallax.step(w, 'vertical', 90), false, 'inside the throttle window')
    T.eq(env.parallax.step(w, 'vertical', 90), false)
    T.eq(w.writes, n, 'no writes for throttled events')
    T.eq(env.parallax.position(w).vertical, 270, 'the position still moved')

    env.advance_ms(50)
    T.eq(env.parallax.tick(w, nil), true, 'a tick writes what was waiting')
    T.eq(w.writes, n + 1)
    T.eq(bg_of(w)[3].vertical_offset, 135, 'factor 0.5 of 270')
    T.eq(env.parallax.tick(w, nil), false, 'and only once')
  end)
end)

T.test('recenter returns the layers to rest in one write', function()
  with_parallax(function(env)
    local w = env.window()
    env.parallax.step(w, 'vertical', 200)
    local n = w.writes
    T.eq(env.parallax.recenter(w), true)
    T.eq(w.writes, n + 1)
    T.eq(w:get_config_overrides().background, nil)
    T.eq(env.parallax.position(w).vertical, 0)
  end)
end)

T.test('toggle_enabled stops the wheel without losing the position', function()
  with_parallax(function(env)
    local w = env.window()
    env.parallax.step(w, 'vertical', 100)
    local enabled = env.parallax.toggle_enabled(w)
    T.eq(enabled, false)
    T.eq(w:get_config_overrides().background, nil, 'layers at rest while disabled')
    env.advance_ms(100)
    T.eq(env.parallax.step(w, 'vertical', 100), false, 'the wheel is ignored')
    T.eq(env.parallax.position(w).vertical, 100)
    env.parallax.toggle_enabled(w)
    T.eq(bg_of(w)[3].vertical_offset, 50, 'back where it was')
  end)
end)

---------------------------------------------------------------------------
-- Pause (R18)
---------------------------------------------------------------------------

T.test('pause then restore: one aggregator write per toggle, parallax offsets intact', function()
  with_parallax(function(env)
    local w = env.window()
    env.parallax.step(w, 'vertical', 100)
    local at_rest = bg_of(w)
    local n = w.writes

    local paused, wrote = env.parallax.toggle_pause(w)
    T.eq(paused, true)
    T.eq(wrote, true)
    T.eq(w.writes, n + 1, 'one write to pause')
    T.eq(#bg_of(w), 1, 'only the base colour remains')
    T.eq(env.parallax.is_paused(w), true)
    T.eq(env.parallax.position(w).vertical, 100, 'the offset is not lost')

    local back, wrote2 = env.parallax.toggle_pause(w)
    T.eq(back, false)
    T.eq(wrote2, true)
    T.eq(w.writes, n + 2, 'one write to restore')
    T.deep_eq(bg_of(w), at_rest, 'the stack returns with its offsets')
  end)
end)

T.test('pause survives a wheel step and a tick without un-pausing', function()
  with_parallax(function(env)
    local w = env.window()
    env.parallax.toggle_pause(w)
    env.advance_ms(100)
    env.parallax.step(w, 'vertical', 90)
    env.parallax.tick(w, { auto_scroll = { enabled = true, speed = 10 } })
    T.eq(#bg_of(w), 1, 'still paused')
    env.parallax.toggle_pause(w)
    T.eq(#bg_of(w), 3)
    T.ok(bg_of(w)[3].vertical_offset > 0, 'and the movement made while paused shows on restore')
  end)
end)

---------------------------------------------------------------------------
-- Auto-scroll (R16)
---------------------------------------------------------------------------

T.test('auto-scroll: exactly one aggregator write per tick, wrapping at the strip height', function()
  with_parallax(function(env)
    local w = env.window()
    local motion = { auto_scroll = { enabled = true, speed = 50, axis = 'vertical' } }
    local before = w.writes
    local offsets = {}
    for i = 1, 25 do
      T.eq(env.parallax.tick(w, motion), true, 'tick ' .. i .. ' wrote')
      T.eq(w.writes, before + i, 'exactly one write per tick')
      offsets[i] = env.overrides.get_channel(w, 'autoscroll').data.vertical
    end
    T.eq(offsets[1], 50)
    T.eq(offsets[9], 450)
    T.eq(offsets[10], 0, '500 wraps to 0 at the 500px strip')
    T.eq(offsets[11], 50)
  end)
end)

T.test('auto-scroll: the horizontal axis wraps at the strip width', function()
  with_parallax(function(env)
    local w = env.window()
    local motion = { auto_scroll = { enabled = true, speed = 400, axis = 'horizontal' } }
    env.parallax.tick(w, motion)
    env.parallax.tick(w, motion)
    env.parallax.tick(w, motion)
    T.eq(env.overrides.get_channel(w, 'autoscroll').data.horizontal, 200, '1200 mod 1000')
    T.eq(env.overrides.get_channel(w, 'autoscroll').data.vertical, 0)
  end)
end)

T.test('auto-scroll off: ticks produce no writes', function()
  with_parallax(function(env)
    local w = env.window()
    for _ = 1, 10 do
      T.eq(env.parallax.tick(w, { auto_scroll = { enabled = false, speed = 50 } }), false)
    end
    for _ = 1, 3 do
      T.eq(env.parallax.tick(w, { auto_scroll = { enabled = true, speed = 0 } }), false, 'speed 0 is off too')
    end
    T.eq(env.parallax.tick(w, nil), false, 'no motion part at all')
    T.eq(w.writes, 0)
  end)
end)

T.test('auto-scroll switched off drops its channel once and then stays quiet', function()
  with_parallax(function(env)
    local w = env.window()
    env.parallax.tick(w, { auto_scroll = { enabled = true, speed = 50 } })
    T.ok(env.overrides.get_channel(w, 'autoscroll'))
    local n = w.writes
    T.eq(env.parallax.tick(w, { auto_scroll = { enabled = false } }), true, 'the layers return to rest')
    T.eq(w.writes, n + 1)
    T.eq(env.overrides.get_channel(w, 'autoscroll'), nil)
    T.eq(env.parallax.tick(w, { auto_scroll = { enabled = false } }), false)
    T.eq(w.writes, n + 1)
  end)
end)

T.test('auto-scroll sums with the wheel offset, and a flush plus an advance is still one write', function()
  with_parallax(function(env)
    local w = env.window()
    env.parallax.step(w, 'vertical', 100)
    env.parallax.step(w, 'vertical', 100) -- throttled, waiting
    local n = w.writes
    env.advance_ms(50)
    T.eq(env.parallax.tick(w, { auto_scroll = { enabled = true, speed = 100, axis = 'vertical' } }), true)
    T.eq(w.writes, n + 1, 'two channels moved, one aggregator write')
    -- wheel 200 + auto 100 = 300 virtual px at factor 0.5.
    T.eq(bg_of(w)[3].vertical_offset, 150)
  end)
end)

T.test('auto-scroll state lives in GLOBAL: a fresh Lua state continues from the same position', function()
  with_parallax(function(env)
    local w = env.window()
    local motion = { auto_scroll = { enabled = true, speed = 50 } }
    env.parallax.tick(w, motion)
    env.parallax.tick(w, motion)

    env.stub.fresh_state()
    forget()
    package.loaded.wezterm = env.stub.wezterm
    local parallax2 = require 'wzt.parallax'
    local overrides2 = require 'wzt.overrides'
    parallax2.tick(w, motion)
    T.eq(overrides2.get_channel(w, 'autoscroll').data.vertical, 150)
  end)
end)

---------------------------------------------------------------------------
-- Interaction with the other channels
---------------------------------------------------------------------------

T.test('a preview and parallax together: clearing the preview keeps the offset', function()
  with_parallax(function(env)
    local w = env.window()
    env.parallax.step(w, 'vertical', 100)
    env.overrides.preview_set(w, { config = { color_scheme = 'Picker' } }, { owner = 'picker' })
    T.eq(w:get_config_overrides().color_scheme, 'Picker')
    T.ok(bg_of(w))
    env.overrides.preview_clear(w)
    T.eq(w:get_config_overrides().color_scheme, nil)
    T.eq(bg_of(w)[3].vertical_offset, 50)
  end)
end)

T.test('actions: the callbacks drive the same functions', function()
  with_parallax(function(env)
    local w = env.window()
    local actions = env.parallax.actions()
    actions.toggle_pause.callback(w, nil)
    T.eq(env.parallax.is_paused(w), true)
    actions.toggle_pause.callback(w, nil)
    T.eq(env.parallax.is_paused(w), false)
    for _, m in ipairs(env.parallax.mouse_bindings({ alt_wheel_scroll = { vertical = true } })) do
      if m.id == 'parallax-vertical-down' then
        m.binding.action.callback(w, nil)
      end
    end
    T.eq(env.parallax.position(w).vertical, env.parallax.WHEEL_PX)
  end)
end)
