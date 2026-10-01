-- Preview OSC protocol tests (U12 / AE3).

local T = ...
local stub_lib = T.stub

local function base_with_layers()
  return {
    generation = 1,
    owned = {},
    background = {
      base_color = '#0c0c18',
      layers = {
        {
          id = 'stars',
          opacity = 1,
          parallax = { vertical = 0.1, horizontal = 0 },
          source = { File = '/tmp/stars.png' },
        },
      },
    },
  }
end

--- Fresh stub + reloaded modules so GLOBAL behaves like a re-eval.
local function with_preview(fn)
  local stub = stub_lib.new()
  package.loaded['wezterm'] = stub.wezterm
  package.loaded['wzt.platform'] = nil
  package.loaded['wzt.apply'] = nil
  package.loaded['wzt.overrides'] = nil
  package.loaded['wzt.preview'] = nil
  local ov = require 'wzt.overrides'
  local preview = require 'wzt.preview'
  local ok, err = pcall(fn, stub, ov, preview)
  stub.cleanup()
  package.loaded['wezterm'] = nil
  package.loaded['wzt.platform'] = nil
  package.loaded['wzt.apply'] = nil
  package.loaded['wzt.overrides'] = nil
  package.loaded['wzt.preview'] = nil
  if not ok then
    error(err, 0)
  end
end

local function ctx_from_presets(map)
  return {
    preview_payload = function(id)
      local p = map[id]
      if not p then
        return nil, 'missing ' .. tostring(id)
      end
      return p
    end,
  }
end

-- AE3: moving across three presets sends three previews; Escape cancel
-- restores pre-preview channels.
T.test('AE3: three presets preview then cancel restores parallax-only', function()
  with_preview(function(stub, ov, preview)
    local window = stub.new_window()
    local pane = stub.new_pane(window)
    ov.set_base(base_with_layers())
    ov.set_channel(window, 'parallax', { vertical = 20, horizontal = 0 })

    local payloads = {
      ['builtin:a'] = { config = { color_scheme = 'A' } },
      ['builtin:b'] = { config = { color_scheme = 'B' } },
      ['builtin:c'] = { config = { color_scheme = 'C' } },
    }
    local ctx = ctx_from_presets(payloads)

    for i, id in ipairs({ 'builtin:a', 'builtin:b', 'builtin:c' }) do
      local tag = preview.handle(window, pane, preview.VAR_PREVIEW, string.format(
        '{"seq":%d,"expires_in":3,"preset_id":"%s"}', i, id
      ), ctx)
      T.eq(tag, 'preview')
      T.eq(window:get_config_overrides().color_scheme, payloads[id].config.color_scheme)
    end
    T.eq(ov.get_channel(window, 'preview').seq, 3)

    preview.handle(window, pane, preview.VAR_CANCEL, '{"seq":4}', ctx)
    T.eq(ov.get_channel(window, 'preview'), nil, 'cancel clears preview')
    T.ok(window:get_config_overrides().background, 'parallax channel still composed')
    T.eq(window:get_config_overrides().color_scheme, nil)
  end)
end)

T.test('expired TUI preview cleared on tick; heartbeat keeps alive', function()
  with_preview(function(stub, ov, preview)
    local window = stub.new_window()
    local pane = stub.new_pane(window)
    ov.set_base(base_with_layers())
    local ctx = ctx_from_presets({
      ['builtin:x'] = { config = { color_scheme = 'X' } },
    })

    preview.handle(window, pane, preview.VAR_PREVIEW,
      '{"seq":1,"expires_in":3,"preset_id":"builtin:x"}', ctx)
    T.ok(ov.get_channel(window, 'preview'))

    -- Force expiry into the past without going through preview_set's clock.
    local ch = ov.get_channel(window, 'preview')
    ov.set_channel(window, 'preview', ch.data, {
      owner = 'tui',
      seq = 1,
      expires_at = 0,
    })
    ov.tick(window)
    T.eq(ov.get_channel(window, 'preview'), nil, 'expired cleared')

    -- Fresh preview, then heartbeat after we push expires_at into the near past
    -- but renew before tick.
    preview.handle(window, pane, preview.VAR_PREVIEW,
      '{"seq":2,"expires_in":3,"preset_id":"builtin:x"}', ctx)
    preview.handle(window, pane, preview.VAR_HEARTBEAT, '{"seq":2,"expires_in":3}', ctx)
    local after = ov.get_channel(window, 'preview')
    T.ok(after and after.expires_at and after.expires_at > 0, 'heartbeat renewed expiry')
    ov.tick(window)
    T.ok(ov.get_channel(window, 'preview'), 'heartbeat kept preview alive')
  end)
end)

T.test('older sequence number is ignored', function()
  with_preview(function(stub, ov, preview)
    local window = stub.new_window()
    local pane = stub.new_pane(window)
    ov.set_base(base_with_layers())
    local ctx = ctx_from_presets({
      ['builtin:new'] = { config = { color_scheme = 'New' } },
      ['builtin:old'] = { config = { color_scheme = 'Old' } },
    })
    preview.handle(window, pane, preview.VAR_PREVIEW,
      '{"seq":5,"preset_id":"builtin:new"}', ctx)
    preview.handle(window, pane, preview.VAR_PREVIEW,
      '{"seq":3,"preset_id":"builtin:old"}', ctx)
    T.eq(window:get_config_overrides().color_scheme, 'New')
    T.eq(ov.get_channel(window, 'preview').seq, 5)
  end)
end)

T.test('two windows previewing affect only their own window', function()
  with_preview(function(stub, ov, preview)
    local w1 = stub.new_window()
    local w2 = stub.new_window()
    local p1 = stub.new_pane(w1)
    local p2 = stub.new_pane(w2)
    ov.set_base(base_with_layers())
    local ctx = ctx_from_presets({
      ['builtin:one'] = { config = { color_scheme = 'One' } },
      ['builtin:two'] = { config = { color_scheme = 'Two' } },
    })
    preview.handle(w1, p1, preview.VAR_PREVIEW,
      '{"seq":1,"preset_id":"builtin:one"}', ctx)
    preview.handle(w2, p2, preview.VAR_PREVIEW,
      '{"seq":1,"preset_id":"builtin:two"}', ctx)
    T.eq(w1:get_config_overrides().color_scheme, 'One')
    T.eq(w2:get_config_overrides().color_scheme, 'Two')
    preview.handle(w1, p1, preview.VAR_CANCEL, '{"seq":2}', ctx)
    T.eq(w1:get_config_overrides().color_scheme, nil)
    T.eq(w2:get_config_overrides().color_scheme, 'Two', 'other window untouched')
  end)
end)

T.test('probe sends APC ack via pane:send_text', function()
  with_preview(function(stub, _ov, preview)
    local window = stub.new_window()
    local pane = stub.new_pane(window)
    local tag = preview.handle(window, pane, preview.VAR_PROBE, '{"seq":42}', {})
    T.eq(tag, 'ack')
    T.eq(#pane.sent, 1)
    T.eq(pane.sent[1], preview.encode_ack(42))
    T.ok(pane.sent[1]:find('\27_wzt;ack=42\27\\', 1, true), 'APC framed')
  end)
end)
