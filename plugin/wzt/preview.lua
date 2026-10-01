-- OSC preview protocol speaker.
--
-- The TUI writes WezTerm OSC 1337 SetUserVar escapes; WezTerm decodes the
-- base64 and fires `user-var-changed` with the UTF-8 JSON body. This module
-- turns those events into aggregator calls. Expiry rules live in
-- overrides.lua; here we only decode, dispatch, and send the APC ack token
-- via pane:send_text so the TUI can confirm it is talking to WezTerm.
--
-- User-var names (must match crates/wzt-preview/src/osc.rs):
--   wzt_probe    handshake; reply with APC ack
--   wzt_preview  candidate preset (full parts or preset_id)
--   wzt_hb       renew expiry on the existing TUI preview
--   wzt_cancel   drop the preview channel
--
-- Ack frame: ESC _ wzt;ack=<seq> ESC \

local wezterm = require 'wezterm'
local overrides = require 'wzt.overrides'

local M = {}

M.VAR_PROBE = 'wzt_probe'
M.VAR_PREVIEW = 'wzt_preview'
M.VAR_HEARTBEAT = 'wzt_hb'
M.VAR_CANCEL = 'wzt_cancel'

M.DEFAULT_EXPIRY = overrides.PREVIEW_EXPIRY or 3

---------------------------------------------------------------------------
-- JSON / ack helpers
---------------------------------------------------------------------------

local function decode_json(text)
  if type(text) ~= 'string' or text == '' then
    return nil, 'empty'
  end
  local ok, doc = pcall(wezterm.json_parse, text)
  if ok and type(doc) == 'table' then
    return doc
  end
  if wezterm.serde and wezterm.serde.json_decode then
    ok, doc = pcall(wezterm.serde.json_decode, text)
    if ok and type(doc) == 'table' then
      return doc
    end
  end
  return nil, 'json'
end

--- APC-framed ack token written into the TUI pane.
function M.encode_ack(seq)
  return string.format('\27_wzt;ack=%s\27\\', tostring(seq))
end

function M.send_ack(pane, seq)
  if not pane or type(pane.send_text) ~= 'function' then
    return false
  end
  local ok = pcall(function()
    pane:send_text(M.encode_ack(seq))
  end)
  return ok
end

---------------------------------------------------------------------------
-- Preview application
---------------------------------------------------------------------------

--- Build {config, background} from a decoded preview message using the
--- engine's `preview_payload` (preset_id) or a caller-supplied builder for
--- raw parts. `ctx.preview_payload(id)` comes from init.lua.
local function payload_for(msg, ctx)
  if type(msg.parts) == 'table' and ctx and type(ctx.parts_payload) == 'function' then
    return ctx.parts_payload(msg.parts, msg.preset_id)
  end
  if type(msg.preset_id) == 'string' and ctx and type(ctx.preview_payload) == 'function' then
    return ctx.preview_payload(msg.preset_id)
  end
  -- Minimal fallback used in unit tests: treat config keys on the message
  -- itself, or a nested `config` field, as the channel data.
  if type(msg.config) == 'table' then
    return { config = msg.config, background = msg.background }
  end
  return nil, 'no preset_id or parts'
end

--- Handle one user-var-changed delivery. Returns a tag string for tests:
--- 'ack' | 'preview' | 'heartbeat' | 'cancel' | 'ignore' | 'error'.
function M.handle(window, pane, name, value, ctx)
  ctx = ctx or {}
  if name == M.VAR_PROBE then
    local msg = decode_json(value)
    local seq = msg and msg.seq
    if seq ~= nil then
      M.send_ack(pane, seq)
      return 'ack'
    end
    return 'error'
  end

  if name == M.VAR_CANCEL then
    overrides.preview_clear(window)
    return 'cancel'
  end

  if name == M.VAR_HEARTBEAT then
    local msg = decode_json(value) or {}
    local ok = overrides.preview_set(window, nil, {
      owner = 'tui',
      seq = msg.seq,
      expires_in = msg.expires_in or M.DEFAULT_EXPIRY,
    })
    -- preview_set returns false on heartbeat (no override rewrite); still success.
    return ok and 'heartbeat' or 'heartbeat'
  end

  if name == M.VAR_PREVIEW then
    local msg, err = decode_json(value)
    if not msg then
      return 'error'
    end
    local data, why = payload_for(msg, ctx)
    if not data then
      wezterm.log_warn('wezterminator preview: ' .. tostring(why or err))
      return 'error'
    end
    overrides.preview_set(window, data, {
      owner = 'tui',
      seq = msg.seq,
      expires_in = msg.expires_in or M.DEFAULT_EXPIRY,
    })
    return 'preview'
  end

  return 'ignore'
end

---------------------------------------------------------------------------
-- Setup
---------------------------------------------------------------------------

--- Register the user-var-changed handler. `ctx` may carry:
---   preview_payload(preset_id) -> {config, background} | nil, reason
---   parts_payload(parts, preset_id) -> same (optional; U13 authoring)
--- Called once per config evaluation (handlers do not survive a reload).
function M.setup(ctx)
  ctx = ctx or {}
  wezterm.on('user-var-changed', function(window, pane, name, value)
    if name == M.VAR_PROBE
      or name == M.VAR_PREVIEW
      or name == M.VAR_HEARTBEAT
      or name == M.VAR_CANCEL
    then
      M.handle(window, pane, name, value, ctx)
    end
  end)
end

return M
