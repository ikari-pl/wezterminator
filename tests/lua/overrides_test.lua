-- Override aggregator tests. Uses the wezterm stub so channels live in GLOBAL
-- the way they do across a real set_config_overrides re-evaluation.

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
        {
          id = 'grid',
          opacity = 1,
          parallax = { vertical = 0.5, horizontal = 0 },
          source = { File = '/tmp/grid.png' },
        },
      },
    },
  }
end

--- Install a fresh stub and reload modules that capture `wezterm` at require time.
local function with_overrides(fn)
  local stub = stub_lib.new()
  package.loaded['wezterm'] = stub.wezterm
  package.loaded['wzt.platform'] = nil
  package.loaded['wzt.apply'] = nil
  package.loaded['wzt.overrides'] = nil
  local ov = require 'wzt.overrides'
  local ok, err = pcall(fn, stub, ov)
  stub.cleanup()
  package.loaded['wezterm'] = nil
  package.loaded['wzt.platform'] = nil
  package.loaded['wzt.apply'] = nil
  package.loaded['wzt.overrides'] = nil
  if not ok then
    error(err, 0)
  end
end

T.test('compose: parallax and preview combine; clearing preview leaves parallax', function()
  with_overrides(function(_, ov)
    local base = base_with_layers()
    local with_both = ov.compose({
      preview = { data = { config = { color_scheme = 'Preview' } } },
      parallax = { data = { vertical = 40, horizontal = 0 } },
    }, base)
    T.eq(with_both.color_scheme, 'Preview')
    T.ok(with_both.background, 'background present with offset')

    local after_clear = ov.compose({
      parallax = { data = { vertical = 40, horizontal = 0 } },
    }, base)
    T.eq(after_clear.color_scheme, nil)
    T.ok(after_clear.background, 'parallax alone still writes background')
  end)
end)

T.test('compose: identical compositions compare equal under same()', function()
  with_overrides(function(_, ov)
    local base = base_with_layers()
    local channels = { parallax = { data = { vertical = 10, horizontal = 2 } } }
    T.ok(ov.same(ov.compose(channels, base), ov.compose(channels, base)))
  end)
end)

T.test('compose: pause changes the stack; restoring keeps parallax offset', function()
  with_overrides(function(_, ov)
    local base = base_with_layers()
    local paused = ov.compose({
      pause = { data = { paused = true } },
      parallax = { data = { vertical = 20, horizontal = 0 } },
    }, base)
    local restored = ov.compose({
      parallax = { data = { vertical = 20, horizontal = 0 } },
    }, base)
    T.ok(paused.background and restored.background)
    T.ok(not ov.same(paused.background, restored.background), 'pause changes the stack')
  end)
end)

T.test('apply: identical composition results in a single set_config_overrides write', function()
  with_overrides(function(stub, ov)
    local window = stub.new_window()
    ov.set_base(base_with_layers())
    ov.set_channel(window, 'parallax', { vertical = 12, horizontal = 0 })
    ov.preview_set(window, { config = { color_scheme = 'A' } }, {})
    ov.apply(window)
    local n1 = window.writes
    ov.apply(window)
    T.eq(window.writes, n1, 'identical composition writes once')
    ov.preview_clear(window)
    ov.apply(window)
    T.eq(window:get_config_overrides().color_scheme, nil)
  end)
end)

T.test('tick: TUI preview with past expiry clears; picker preview without expiry stays', function()
  with_overrides(function(stub, ov)
    local window = stub.new_window()
    ov.set_base(base_with_layers())
    -- Force an already-expired TUI preview via set_channel (preview_set
    -- always computes expires_at from now when owner is tui).
    ov.set_channel(window, 'preview', { config = { color_scheme = 'TUI' } }, {
      owner = 'tui',
      expires_at = 0,
    })
    ov.tick(window)
    T.eq(ov.get_channel(window, 'preview'), nil, 'expired TUI preview cleared')

    ov.preview_set(window, { config = { color_scheme = 'Picker' } }, { owner = 'picker' })
    ov.tick(window)
    local ch = ov.get_channel(window, 'preview')
    T.ok(ch and ch.data and ch.data.config.color_scheme == 'Picker', 'picker preview stays')
  end)
end)
