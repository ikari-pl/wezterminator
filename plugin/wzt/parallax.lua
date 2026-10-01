-- Wheel-driven virtual parallax, auto-scroll and background pause. Every one
-- of them is a CHANNEL of the override aggregator (overrides.lua); none calls
-- set_config_overrides itself. That is the fix for the metis bug where a
-- parallax tick wiped an applied theme: two writers, one table, last writer wins.
--
--   channel `parallax`    {vertical, horizontal}  virtual scroll from ALT+wheel
--   channel `autoscroll`  {vertical, horizontal}  motion; summed with parallax
--   channel `pause`       {paused = true}          layers collapse to base colour
--
-- WHY A VIRTUAL SCROLL: WezTerm's native `Parallax` attachment keys off the
-- scrollback viewport offset. Full-screen applications (editors, pagers, AI
-- CLIs) repaint in place, so the viewport never moves and the background sits
-- still. This module synthesises the missing signal from wheel events.
--
-- WHY A MODIFIER: applications that enable mouse reporting receive wheel
-- events directly, and WezTerm has no action to forward a mouse event onward.
-- Intercepting the plain wheel would swallow the application's scrolling with
-- no way to give it back. ALT+wheel is used by no terminal application, so
-- consuming it breaks nothing and nothing needs forwarding. ALT+SHIFT+wheel
-- (which macOS delivers as WheelLeft/WheelRight) moves the horizontal axis.
--
-- SLACK: how far a layer may drift from its artwork is clamped by the
-- aggregator (apply.SLACK), per layer, after the layer's own parallax factor.
-- RANGE below only bounds the virtual position so it cannot wind up unbounded.
--
-- AUTO-SCROLL: speed is pixels per TICK, wrapping at the strip width (the art
-- resolution on that axis). When enabled, ticks are driven by
-- `wezterm.time.call_after` on the focused window only (not the 1 Hz
-- `update-status` heartbeat). Each tick costs at most ONE aggregator write,
-- and none when auto-scroll is off. `M.tick` is the unit under test;
-- `M.setup` only schedules it.

local wezterm = require 'wezterm'
local platform = require 'wzt.platform'
local overrides = require 'wzt.overrides'

local M = {}

local STORE_KEY = 'wzt_parallax'

--- Virtual pixels per wheel notch. Higher is livelier.
M.WHEEL_PX = 90
--- Virtual range in pixels, either side of rest.
M.RANGE = 1300
--- Do not rewrite overrides more often than this for wheel events (ms). Every
--- write re-evaluates the whole config, and wheels can outrun the display.
M.THROTTLE_MS = 24
--- Strip size when the art resolution is unknown.
M.DEFAULT_STRIP = { horizontal = 1920, vertical = 1080 }
--- Auto-scroll frame interval in seconds (starting guess from the plan; U10
--- measures the per-write cost on metis).
M.TICK_S = 0.05

---------------------------------------------------------------------------
-- Helpers
---------------------------------------------------------------------------

local function now_ms()
  if M._now_ms then
    return M._now_ms()
  end
  local ok, v = pcall(function()
    return math.floor(tonumber(wezterm.time.now():format('%s%f'):sub(1, 13)))
  end)
  if ok and v then
    return v
  end
  return os.time() * 1000
end

local function clamp(v)
  return math.max(-M.RANGE, math.min(M.RANGE, v))
end

local function window_key(window)
  return tostring(window:window_id())
end

local function load_all()
  return platform.store_get(STORE_KEY) or {}
end

local function toast(window, text)
  pcall(function()
    window:toast_notification('wezterminator', text, nil, 1500)
  end)
end

--- Strip width on `axis`, from the art resolution the base stack was built for.
function M.strip_size(axis)
  local base = overrides.get_base()
  local res = base and base.background and base.background.resolution
  local n = res and (axis == 'horizontal' and res.w or res.h)
  if type(n) == 'number' and n > 0 then
    return n
  end
  return M.DEFAULT_STRIP[axis] or M.DEFAULT_STRIP.vertical
end

---------------------------------------------------------------------------
-- Wheel
---------------------------------------------------------------------------

--- Move the virtual position by `delta` pixels on `axis`. Returns true when
--- overrides were written; false when disabled, throttled (the position is
--- kept and the next tick writes it) or the result composes to no change.
function M.step(window, axis, delta)
  local all = load_all()
  local key = window_key(window)
  local rec = all[key] or {}
  if rec.disabled then
    return false
  end
  local pos = rec.pos or { vertical = 0, horizontal = 0 }
  pos[axis] = clamp((pos[axis] or 0) + delta)
  rec.pos = pos

  local t = now_ms()
  local wrote = false
  if rec.last_ms == nil or t - rec.last_ms >= M.THROTTLE_MS then
    rec.last_ms = t
    rec.dirty = false
    all[key] = rec
    platform.store_set(STORE_KEY, all)
    wrote = overrides.set_channel(window, 'parallax', pos)
  else
    rec.dirty = true
    all[key] = rec
    platform.store_set(STORE_KEY, all)
  end
  return wrote
end

--- The virtual position of a window: { vertical, horizontal }.
function M.position(window)
  local rec = load_all()[window_key(window)]
  local pos = rec and rec.pos or {}
  return { vertical = pos.vertical or 0, horizontal = pos.horizontal or 0 }
end

--- Return the background to rest. Auto-scroll keeps running from where it is.
function M.recenter(window)
  local all = load_all()
  local key = window_key(window)
  local rec = all[key] or {}
  rec.pos = { vertical = 0, horizontal = 0 }
  rec.dirty = false
  all[key] = rec
  platform.store_set(STORE_KEY, all)
  return overrides.clear_channel(window, 'parallax')
end

--- Turn the ALT+wheel effect on or off for a window. The position is kept.
--- Returns the new enabled state and whether overrides were written.
function M.toggle_enabled(window)
  local all = load_all()
  local key = window_key(window)
  local rec = all[key] or {}
  rec.disabled = not rec.disabled
  all[key] = rec
  platform.store_set(STORE_KEY, all)

  local wrote
  if rec.disabled then
    wrote = overrides.clear_channel(window, 'parallax')
  else
    local pos = rec.pos or { vertical = 0, horizontal = 0 }
    wrote = overrides.set_channel(window, 'parallax', pos)
  end
  toast(window, 'Virtual parallax ' .. (rec.disabled and 'off' or 'on'))
  return not rec.disabled, wrote
end

---------------------------------------------------------------------------
-- Background pause (R18)
---------------------------------------------------------------------------

--- Pause or restore the layer stack. The parallax and auto-scroll channels
--- are untouched, so restoring brings the layers back where they were.
--- Returns paused, wrote. One aggregator write per toggle.
function M.toggle_pause(window)
  local ch = overrides.get_channel(window, 'pause')
  local was = ch and ch.data and ch.data.paused or false
  local wrote
  if was then
    wrote = overrides.clear_channel(window, 'pause')
  else
    wrote = overrides.set_channel(window, 'pause', { paused = true })
  end
  toast(window, was and 'Background restored' or 'Background paused')
  return not was, wrote
end

function M.is_paused(window)
  local ch = overrides.get_channel(window, 'pause')
  return ch ~= nil and ch.data ~= nil and ch.data.paused == true
end

---------------------------------------------------------------------------
-- Auto-scroll tick
---------------------------------------------------------------------------

--- Flush a throttled wheel position without advancing auto-scroll. Used by the
--- 1 Hz status tick while the call_after loop owns auto-scroll.
function M.flush_dirty(window)
  local all = load_all()
  local key = window_key(window)
  local rec = all[key] or {}
  if not rec.dirty or rec.disabled then
    return false
  end
  rec.dirty = false
  rec.last_ms = now_ms()
  all[key] = rec
  platform.store_set(STORE_KEY, all)
  return overrides.set_channel(window, 'parallax', rec.pos or { vertical = 0, horizontal = 0 })
end

--- Advance one tick. `motion` is the resolved preset's motion part. Also
--- flushes a wheel position that was throttled. At most one aggregator write;
--- none when auto-scroll is off and nothing is waiting. Returns true when
--- overrides were written.
function M.tick(window, motion)
  local all = load_all()
  local key = window_key(window)
  local rec = all[key] or {}
  local updates, changed = {}, false

  if rec.dirty and not rec.disabled then
    updates.parallax = rec.pos or { vertical = 0, horizontal = 0 }
    rec.dirty = false
    rec.last_ms = now_ms()
    changed = true
  end

  local auto = motion and motion.auto_scroll
  if auto and auto.enabled and (tonumber(auto.speed) or 0) > 0 then
    local axis = auto.axis == 'horizontal' and 'horizontal' or 'vertical'
    local pos = rec.auto or { vertical = 0, horizontal = 0 }
    pos[axis] = ((pos[axis] or 0) + tonumber(auto.speed)) % M.strip_size(axis)
    rec.auto = pos
    updates.autoscroll = { vertical = pos.vertical, horizontal = pos.horizontal }
    changed = true
  elseif rec.auto then
    -- Switched off (commit to a preset with auto-scroll off): drop the channel once.
    rec.auto = nil
    updates.autoscroll = false
    changed = true
  end

  if not changed then
    return false
  end
  all[key] = rec
  platform.store_set(STORE_KEY, all)
  return overrides.set_channels(window, updates)
end

local function auto_enabled(motion)
  local auto = motion and motion.auto_scroll
  return auto and auto.enabled and (tonumber(auto.speed) or 0) > 0
end

local function focused_windows()
  if not (wezterm.gui and type(wezterm.gui.gui_windows) == 'function') then
    return {}
  end
  local ok, list = pcall(wezterm.gui.gui_windows)
  if not ok or type(list) ~= 'table' then
    return {}
  end
  local out = {}
  for _, window in ipairs(list) do
    local fok, focused = pcall(function()
      return window:is_focused()
    end)
    -- Builds without is_focused treat every window as focused.
    if not fok or focused then
      out[#out + 1] = window
    end
  end
  return out
end

--- Schedule the next auto-scroll frame. Exposed for tests that drive the timer
--- without waiting on real wall clock.
function M.schedule(motion)
  if not auto_enabled(motion) then
    return false
  end
  if not (wezterm.time and type(wezterm.time.call_after) == 'function') then
    return false
  end
  wezterm.time.call_after(M.TICK_S, function()
    for _, window in ipairs(focused_windows()) do
      M.tick(window, motion)
    end
    M.schedule(motion)
  end)
  return true
end

---------------------------------------------------------------------------
-- Actions and bindings
---------------------------------------------------------------------------

--- Callback actions by name, for keys.lua and the palette. Built on demand
--- because wezterm.action_callback needs the live module.
function M.actions()
  return {
    toggle_pause = wezterm.action_callback(function(window)
      M.toggle_pause(window)
    end),
    toggle_parallax = wezterm.action_callback(function(window)
      M.toggle_enabled(window)
    end),
    recenter_parallax = wezterm.action_callback(function(window)
      M.recenter(window)
    end),
  }
end

local function wheel_action(axis, delta)
  return wezterm.action_callback(function(window)
    M.step(window, axis, delta)
  end)
end

--- Mouse bindings for the resolved motion part, as { {id =, binding =}, ... }.
--- The `id`s are for conflict reports; `binding` is what goes in
--- config.mouse_bindings.
---   alt_wheel_scroll.vertical    ALT + WheelUp / WheelDown
---   alt_wheel_scroll.horizontal  ALT|SHIFT + WheelUp / WheelDown and WheelLeft / WheelRight
function M.mouse_bindings(motion)
  local scroll = motion and motion.alt_wheel_scroll or {}
  local out = {}
  local function add(id, button, mods, axis, delta)
    out[#out + 1] = {
      id = id,
      binding = {
        event = { Down = { streak = 1, button = { [button] = 1 } } },
        mods = mods,
        -- Fire in full-screen apps too; the native Parallax attachment fails
        -- in exactly the place this exists for.
        alt_screen = 'Any',
        mouse_reporting = true,
        action = wheel_action(axis, delta),
      },
    }
  end
  if scroll.vertical ~= false then
    add('parallax-vertical-up', 'WheelUp', 'ALT', 'vertical', -M.WHEEL_PX)
    add('parallax-vertical-down', 'WheelDown', 'ALT', 'vertical', M.WHEEL_PX)
  end
  if scroll.horizontal == true then
    add('parallax-horizontal-up', 'WheelUp', 'ALT|SHIFT', 'horizontal', -M.WHEEL_PX)
    add('parallax-horizontal-down', 'WheelDown', 'ALT|SHIFT', 'horizontal', M.WHEEL_PX)
    add('parallax-horizontal-left', 'WheelLeft', 'ALT|SHIFT', 'horizontal', -M.WHEEL_PX)
    add('parallax-horizontal-right', 'WheelRight', 'ALT|SHIFT', 'horizontal', M.WHEEL_PX)
  end
  return out
end

--- Register handlers. `ctx.motion` is the resolved motion part.
--- Auto-scroll uses `wezterm.time.call_after` on focused windows only.
--- `update-status` still flushes a throttled wheel position (and, when
--- auto-scroll is off, runs `M.tick` so a switched-off channel is cleared).
function M.setup(ctx)
  local motion = ctx.motion
  local auto_on = auto_enabled(motion)
  wezterm.on('update-status', function(window)
    if auto_on then
      M.flush_dirty(window)
    else
      M.tick(window, motion)
    end
  end)
  if auto_on then
    M.schedule(motion)
  end
end

return M
