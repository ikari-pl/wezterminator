-- The ONE module that calls set_config_overrides.
--
-- Why one: set_config_overrides replaces a window's whole override table, so
-- two modules each writing "their" keys wipe each other (metis's parallax.lua
-- did exactly that to an applied theme on the next ALT+wheel tick). Here every
-- contributor owns a named CHANNEL; the aggregator composes the channels,
-- compares the result with what it last applied, and writes only on change.
--
-- Why GLOBAL: every set_config_overrides call re-evaluates the whole config in
-- a fresh Lua state, so module locals do not survive. Channels, the last
-- applied table, preview expiry and the generation a window has seen all live
-- in wezterm.GLOBAL, keyed by window id. Values there are plain data only (no
-- functions), and every update reads a top-level key, changes the copy and
-- writes it back.
--
-- CHANNELS (composition order, low to high):
--   preview    {config = {<config key> = value, ...}, background = <apply.background result>}
--              Window-local, never persisted. Two owners with different rules
--              about expiry -- see below.
--   parallax   {vertical = px, horizontal = px}   virtual scroll from ALT+wheel
--   autoscroll {vertical = px, horizontal = px}   motion; summed with parallax
--   pause      {paused = true}                    background collapses to base colour
-- Offsets and pause only matter to the background, so they are composed into
-- one `background` value from the preset's layer stack and its per-layer
-- parallax factors. That stack is the BASE, written by init.lua at every
-- evaluation; a commit changes it, and the offsets then apply to the new one.
--
-- PREVIEW EXPIRY (owned here; preview.lua, U12, only speaks the OSC protocol):
--   * A preview from the TUI carries `expires_at`. The TUI renews it with a
--     heartbeat; `tick` (called on update-status) clears an expired one, so a
--     crashed TUI's window reverts within the expiry window.
--   * A preview from an in-WezTerm Lua picker (InputSelector) has NO expiry.
--     The picker clears it when the selector closes.
--
-- RELOADS: WezTerm keeps each window's overrides across a full reload, and
-- fires window-config-reloaded for set_config_overrides too. A window records
-- the generation it has rebuilt for. Only a NEW generation (state or a layer
-- file changed) drops the preview and rewrites overrides; an event caused by
-- our own write sees the same generation and does nothing, which is what
-- prevents a reload loop.

local wezterm = require 'wezterm'
local apply = require 'wzt.apply'
local platform = require 'wzt.platform'

local M = {}

local WINDOWS_KEY = 'wzt_windows'
local BASE_KEY = 'wzt_base'
local TOAST_KEY = 'wzt_toast'

--- Preview window in seconds. The TUI heartbeat (1 s) must be shorter.
M.PREVIEW_EXPIRY = 3

---------------------------------------------------------------------------
-- Helpers
---------------------------------------------------------------------------

-- Equality that treats every empty table alike: arrays lose their markers
-- going through wezterm.GLOBAL, and `{}` versus `[]` makes no difference to an
-- override table.
local function same(a, b)
  if a == b then
    return true
  end
  if type(a) ~= 'table' or type(b) ~= 'table' then
    return false
  end
  for k, v in pairs(a) do
    if not same(v, b[k]) then
      return false
    end
  end
  for k in pairs(b) do
    if a[k] == nil then
      return false
    end
  end
  return true
end
M.same = same

local function window_key(window)
  return tostring(window:window_id())
end

local function load_windows()
  return platform.store_get(WINDOWS_KEY) or {}
end

local function save_windows(ws)
  platform.store_set(WINDOWS_KEY, ws)
end

local function record(ws, key)
  local rec = ws[key] or {}
  rec.channels = rec.channels or {}
  ws[key] = rec
  return rec
end

---------------------------------------------------------------------------
-- The base (what the preset resolved to, as of the latest evaluation)
---------------------------------------------------------------------------

--- Called by init.lua on every evaluation. base = {
---   generation, background (apply.background result) or nil,
---   owned = {config keys the user's config set, add-on mode},
---   notices = {...}, active_id, ...
--- }
--- Written only when it differs from what is stored.
function M.set_base(base)
  local current = platform.store_get(BASE_KEY)
  if not same(current, base) then
    platform.store_set(BASE_KEY, base)
  end
end

function M.get_base()
  return platform.store_get(BASE_KEY)
end

---------------------------------------------------------------------------
-- Composition
---------------------------------------------------------------------------

local function offset_of(channels, names)
  local v, h = 0, 0
  for _, name in ipairs(names) do
    local ch = channels[name]
    local d = ch and ch.data
    if d then
      v = v + (d.vertical or 0)
      h = h + (d.horizontal or 0)
    end
  end
  return v, h
end

--- Compose a window's channels into an override table. Pure with respect to
--- WezTerm: `channels` and `base` are plain data.
function M.compose(channels, base)
  base = base or {}
  local owned = {}
  for _, k in ipairs(base.owned or {}) do
    owned[k] = true
  end

  local out = {}
  local bg = base.background

  -- In add-on mode the user's own keys are never overridden, preview included.
  local preview = channels.preview and channels.preview.data
  if preview then
    for k, v in pairs(preview.config or {}) do
      if not owned[k] then
        out[k] = v
      end
    end
    if preview.background then
      bg = preview.background
    end
  end

  local v, h = offset_of(channels, { 'parallax', 'autoscroll' })
  local pause = channels.pause and channels.pause.data
  local paused = pause and pause.paused or false

  local wants_background = (preview and preview.background ~= nil) or v ~= 0 or h ~= 0 or paused
  if wants_background and bg and not owned.background then
    out.background = apply.compose_background(bg, { vertical = v, horizontal = h }, paused)
  end
  return out
end

---------------------------------------------------------------------------
-- Writing
---------------------------------------------------------------------------

--- Compose and, if it differs from the last applied table, write it. Returns
--- true when set_config_overrides was called.
function M.apply(window)
  local key = window_key(window)
  local ws = load_windows()
  local rec = record(ws, key)
  local composed = M.compose(rec.channels, M.get_base())
  if same(composed, rec.last_applied or {}) then
    return false
  end
  -- Recorded before the call: if WezTerm re-enters synchronously, the
  -- re-entrant handler already sees the value that was written.
  rec.last_applied = composed
  save_windows(ws)
  window:set_config_overrides(composed)
  return true
end

--- Set a channel and apply. `data` is the channel payload; `meta` may carry
--- `owner`, `seq` and `expires_at`. Returns true when overrides were written.
function M.set_channel(window, name, data, meta)
  local key = window_key(window)
  local ws = load_windows()
  local rec = record(ws, key)
  local ch = { data = platform.plain(data) or {} }
  for _, f in ipairs({ 'owner', 'seq', 'expires_at' }) do
    if meta and meta[f] ~= nil then
      ch[f] = meta[f]
    end
  end
  rec.channels[name] = ch
  save_windows(ws)
  return M.apply(window)
end

function M.get_channel(window, name)
  local rec = load_windows()[window_key(window)]
  return rec and rec.channels and rec.channels[name] or nil
end

function M.clear_channel(window, name)
  local key = window_key(window)
  local ws = load_windows()
  local rec = ws[key]
  if not rec or not rec.channels or rec.channels[name] == nil then
    return false
  end
  rec.channels[name] = nil
  save_windows(ws)
  return M.apply(window)
end

---------------------------------------------------------------------------
-- Preview
---------------------------------------------------------------------------

--- Start or replace a preview.
---   data         {config, background}; nil renews the existing preview (heartbeat)
---   meta.owner   'tui' for OSC previews (they expire); anything else never expires
---   meta.seq     sequence number; a strictly older one than the current is ignored
---   meta.expires_in  seconds (defaults to PREVIEW_EXPIRY for owner 'tui')
--- Returns true when overrides were written.
function M.preview_set(window, data, meta)
  meta = meta or {}
  local existing = M.get_channel(window, 'preview')
  if existing and existing.seq and meta.seq and meta.seq < existing.seq then
    return false
  end

  local expires_at
  if meta.owner == 'tui' then
    expires_at = platform.now() + (meta.expires_in or M.PREVIEW_EXPIRY)
  end

  if data == nil then
    -- Heartbeat: keep the preview, push the expiry out.
    if not existing then
      return false
    end
    local ws = load_windows()
    local ch = record(ws, window_key(window)).channels.preview
    if ch then
      ch.expires_at = expires_at
      if meta.seq then
        ch.seq = meta.seq
      end
      save_windows(ws)
    end
    return false
  end

  return M.set_channel(window, 'preview', data, {
    owner = meta.owner,
    seq = meta.seq,
    expires_at = expires_at,
  })
end

--- Drop the preview, restoring whatever the other channels compose to.
function M.preview_clear(window)
  return M.clear_channel(window, 'preview')
end

---------------------------------------------------------------------------
-- Events
---------------------------------------------------------------------------

local function toast_text(notice, base)
  if notice.code == 'active_preset_missing' then
    return string.format('Preset %s not found; using %s', tostring(notice.requested), tostring(notice.used))
  elseif notice.code == 'theme_missing' then
    return string.format('Preset %s needs missing theme %s', tostring(notice.preset), tostring(notice.theme))
  elseif notice.code == 'no_active_preset' then
    return string.format('No preset chosen yet; using %s', tostring(notice.used))
  end
  return tostring(notice.code)
end

-- Show resolution notices once per generation, in whichever window hears
-- about the reload first.
local function toast_notices(window, base)
  if not base.notices or #base.notices == 0 then
    return false
  end
  local seen = platform.store_get(TOAST_KEY)
  if seen and seen.generation == base.generation then
    return false
  end
  platform.store_set(TOAST_KEY, { generation = base.generation })
  for _, notice in ipairs(base.notices) do
    pcall(function()
      window:toast_notification('wezterminator', toast_text(notice, base), nil, 4000)
    end)
  end
  return true
end

--- window-config-reloaded. Rebuild only when the generation is new to this
--- window; see the RELOADS note at the top. Returns true when it rebuilt.
function M.on_config_reloaded(window)
  local base = M.get_base()
  if not base or base.generation == nil then
    return false
  end
  local key = window_key(window)
  local ws = load_windows()
  local rec = record(ws, key)
  if rec.seen_generation == base.generation then
    return false
  end
  rec.seen_generation = base.generation

  -- A preview is window-local and never survives a commit.
  rec.channels.preview = nil

  local composed = M.compose(rec.channels, base)
  rec.last_applied = composed
  save_windows(ws)

  -- The window may still hold overrides from before the reload. Compare with
  -- what it actually has, so an unchanged window is not written to at all.
  local current = window:get_config_overrides() or {}
  if not same(current, composed) then
    window:set_config_overrides(composed)
  end
  toast_notices(window, base)
  return true
end

--- update-status (about once a second): clear expired TUI previews and forget
--- windows that have closed. Returns true when overrides were written.
function M.tick(window)
  local now = platform.now()
  local key = window_key(window)
  local ws = load_windows()
  local dirty = false

  local rec = ws[key]
  local expired = false
  if rec and rec.channels then
    for name, ch in pairs(rec.channels) do
      if ch.expires_at and ch.expires_at <= now then
        rec.channels[name] = nil
        expired = true
      end
    end
  end

  -- Closed windows: keep only windows WezTerm still lists (plus this one).
  if wezterm.gui and type(wezterm.gui.gui_windows) == 'function' then
    local ok, list = pcall(wezterm.gui.gui_windows)
    if ok and type(list) == 'table' then
      local live = { [key] = true }
      for _, w in ipairs(list) do
        local okid, id = pcall(function()
          return w:window_id()
        end)
        if okid then
          live[tostring(id)] = true
        end
      end
      for k in pairs(ws) do
        if not live[k] then
          ws[k] = nil
          dirty = true
        end
      end
    end
  end

  if expired or dirty then
    save_windows(ws)
  end
  if expired then
    return M.apply(window)
  end
  return false
end

--- Register the event handlers. Called once per evaluation (handlers do not
--- survive a reload, because the Lua state does not).
function M.setup()
  wezterm.on('window-config-reloaded', function(window, _pane)
    M.on_config_reloaded(window)
  end)
  wezterm.on('update-status', function(window, _pane)
    M.tick(window)
  end)
end

return M
