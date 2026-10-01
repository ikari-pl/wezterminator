-- The override aggregator end to end: real plugin evaluation, real built-in
-- themes, window objects that behave like WezTerm's.
--
-- overrides_test.lua covers the pure composition and the write-on-change rule.
-- This file covers what only shows up across evaluations: channels that must
-- survive a fresh Lua state, a commit landing while a preview and a parallax
-- offset are live, a reload that must not loop, and add-on precedence reaching
-- all the way through the aggregator.

local T = ...
local stub_lib = T.stub

local ROOT
do
  local p = io.popen('cd "' .. T.root .. '" && pwd')
  ROOT = p:read('*l')
  p:close()
end

local function preset_json(slug)
  return string.format([[{
  "schema_version": 1, "id": "local:%s", "name": "%s", "based_on": null,
  "parts": {
    "art": {"theme": "local:%s"}, "scheme": {"theme": "local:%s"}, "palette": {"theme": "local:%s"},
    "font": {"preferred": ["Menlo"], "fallback": ["Courier"]},
    "chrome": {"opacity": 1},
    "status": {"style": "pill", "segments": ["clock"]},
    "motion": {}
  }
}]], slug, slug, slug, slug, slug)
end

--- A copy of a built-in theme under a local id.
local function theme_copy(builtin_slug, local_slug)
  local text, n = T.read_file(ROOT .. '/themes/' .. builtin_slug .. '/theme.json')
    :gsub('"id": "builtin:' .. builtin_slug:gsub('%-', '%%-') .. '"', '"id": "local:' .. local_slug .. '"')
  T.eq(n, 1, 'theme id substituted once')
  return text
end

--- Two local presets with shipped art at 1000x500, so layers are real files
--- with parallax factors (fallback layers have none).
--- `mine` is cpc-cool; `other` is ember.
local function setup(stub_opts, plugin_opts)
  local stub = stub_lib.new(stub_opts)
  local tmp = stub.tmpdir()
  local dirs = {
    ['local'] = tmp .. '/config',
    fleet = tmp .. '/data/fleet',
    state = tmp .. '/state',
    data = tmp .. '/data',
  }
  local L = dirs['local']
  stub.write(L .. '/themes/mine/theme.json', theme_copy('cpc-cool', 'mine'))
  stub.write(L .. '/themes/other/theme.json', theme_copy('ember', 'other'))
  stub.write(L .. '/presets/mine.json', preset_json('mine'))
  stub.write(L .. '/presets/other.json', preset_json('other'))
  for _, slug in ipairs({ 'mine', 'other' }) do
    for _, id in ipairs({ 'nebula', 'stars', 'grid', 'haze' }) do
      stub.write(string.format('%s/themes/%s/art/1000x500/%s.png', L, slug, id), '')
    end
  end
  stub.write(dirs.state .. '/screens.json', '{"schema_version": 1, "screens": [{"width": 1000, "height": 500}]}')
  stub.write(dirs.state .. '/state.json', '{"schema_version": 1, "active_preset": "local:mine", "history": []}')

  local opts = { dir = ROOT, dirs = dirs, mode = 'add-on' }
  for k, v in pairs(plugin_opts or {}) do
    opts[k] = v
  end
  local env = { stub = stub, dirs = dirs, opts = opts }
  function env.eval(config)
    return stub.eval(ROOT, opts, config or {})
  end
  function env.mod(name)
    return package.loaded['wzt.' .. name]
  end
  return env
end

local function with_env(stub_opts, plugin_opts, fn)
  return function()
    local env = setup(stub_opts, plugin_opts)
    local ok, err = pcall(fn, env)
    env.stub.cleanup()
    if not ok then
      error(err, 0)
    end
  end
end

--- Offset a layer should carry for virtual scroll `px`.
local function expected_offset(px, factor)
  return math.max(-620, math.min(620, px * factor))
end

local function layer_of(overrides, index)
  return overrides.background[index]
end

local GRID = 4 -- base colour, nebula, stars, grid, haze

---------------------------------------------------------------------------

T.test('fixture sanity: shipped art is found and layers carry parallax factors', with_env(nil, nil, function(env)
  local _, handle = env.eval()
  T.eq(handle.result.active.id, 'local:mine')
  T.eq(handle.background.source, 'shipped')
  T.ok(handle.background.meta[GRID].vertical > 0, 'the grid layer has a parallax factor')
end))

T.test('preview and parallax combine; clearing the preview leaves parallax intact', with_env(nil, nil, function(env)
  local _, handle = env.eval()
  local O = env.mod('overrides')
  local w = env.stub.new_window()
  env.stub.emit_reloaded(w)
  T.eq(w.writes, 0, 'a fresh window with no channels needs no overrides')

  local mine_factor = handle.background.meta[GRID].vertical
  T.eq(O.set_channel(w, 'parallax', { vertical = 200 }), true)
  local o = w:get_config_overrides()
  T.eq(w.writes, 1)
  T.eq(o.colors, nil, 'parallax alone touches only the background')
  T.eq(layer_of(o, GRID).vertical_offset, expected_offset(200, mine_factor))
  T.ok(layer_of(o, GRID).source.File:find('/themes/mine/', 1, true))

  local payload = assert(handle.preview_payload('local:other'))
  T.eq(O.preview_set(w, payload, {}), true)
  o = w:get_config_overrides()
  T.eq(o.colors.tab_bar.active_tab.fg_color, '#fabd2f', 'ember palette from the preview')
  T.ok(layer_of(o, GRID).source.File:find('/themes/other/', 1, true), 'previewed art')
  T.eq(layer_of(o, GRID).vertical_offset, expected_offset(200, payload.background.meta[GRID].vertical),
    'the parallax offset is applied to the previewed layers, not lost')

  T.eq(O.preview_clear(w), true)
  o = w:get_config_overrides()
  T.eq(o.colors, nil, 'preview colours are gone')
  T.ok(layer_of(o, GRID).source.File:find('/themes/mine/', 1, true), 'base art is back')
  T.eq(layer_of(o, GRID).vertical_offset, expected_offset(200, mine_factor), 'parallax survived the preview')
  T.eq(w.writes, 3)
end))

T.test('channels survive a fresh Lua state because they live in wezterm.GLOBAL', with_env(nil, nil, function(env)
  env.eval()
  local w = env.stub.new_window()
  env.mod('overrides').set_channel(w, 'parallax', { vertical = 150 })

  env.eval() -- module locals and handlers are gone
  local fresh = env.mod('overrides')
  local ch = fresh.get_channel(w, 'parallax')
  T.ok(ch ~= nil, 'channel still there')
  T.eq(ch.data.vertical, 150)
  -- Re-applying the same composition is a no-op even though this module
  -- instance has never seen the window.
  T.eq(fresh.apply(w), false)
  T.eq(w.writes, 1)
end))

T.test('a commit while a preview and an offset are live: the reload keeps the offset on the new theme', with_env(nil, nil, function(env)
  local _, handle = env.eval()
  local O = env.mod('overrides')
  local w = env.stub.new_window()
  env.stub.emit_reloaded(w)
  O.set_channel(w, 'parallax', { vertical = 300 })
  O.preview_set(w, assert(handle.preview_payload('local:other')), {})
  local writes_before = w.writes

  -- The TUI commits `other`: it writes state.json, WezTerm reloads in full.
  T.eq(env.mod('state').commit(env.dirs, 'local:other'), true)
  local _, after = env.eval()
  T.eq(after.result.active.id, 'local:other')
  T.eq(after.generation_changed, true)
  env.stub.emit_reloaded(w)

  T.ok(w.writes - writes_before <= 1, 'at most one override write for the window per reload')
  T.eq(O.get_channel(w, 'preview'), nil, 'a preview never survives a commit')
  T.eq(env.mod('overrides').get_channel(w, 'parallax').data.vertical, 300, 'the offset channel does')
  local o = w:get_config_overrides()
  T.eq(o.colors, nil, 'the base config now carries the new colours, so no override is needed')
  T.ok(layer_of(o, GRID).source.File:find('/themes/other/', 1, true), 'new theme layers')
  T.eq(layer_of(o, GRID).vertical_offset, expected_offset(300, after.background.meta[GRID].vertical))
end))

T.test('a window with no channels is not written to after a commit', with_env(nil, nil, function(env)
  env.eval()
  local w = env.stub.new_window()
  env.stub.emit_reloaded(w)
  env.mod('state').commit(env.dirs, 'local:other')
  env.eval()
  env.stub.emit_reloaded(w)
  T.eq(w.writes, 0, 'nothing to override, nothing written: the base config already changed')
end))

T.test('two windows, one commit: one base evaluation and at most one write each', with_env(nil, nil, function(env)
  local _, handle = env.eval()
  local O = env.mod('overrides')
  local w1, w2 = env.stub.new_window(), env.stub.new_window()
  env.stub.emit_reloaded()
  O.set_channel(w1, 'parallax', { vertical = 100 })
  O.set_channel(w2, 'parallax', { vertical = 250 })
  O.preview_set(w2, assert(handle.preview_payload('local:other')), {})
  local b1, b2, evals = w1.writes, w2.writes, env.stub.evals

  env.mod('state').commit(env.dirs, 'local:other')
  env.eval()
  env.stub.emit_reloaded()

  T.eq(env.stub.evals - evals, 1, 'one base evaluation per commit')
  T.ok(w1.writes - b1 <= 1, 'window 1')
  T.ok(w2.writes - b2 <= 1, 'window 2')
  -- Each window kept its OWN offset, now on the committed theme's layers.
  local factor = env.mod('overrides').get_base().background.meta[GRID].vertical
  T.eq(layer_of(w1:get_config_overrides(), GRID).vertical_offset, expected_offset(100, factor))
  T.eq(layer_of(w2:get_config_overrides(), GRID).vertical_offset, expected_offset(250, factor))
end))

T.test('set_config_overrides re-evaluating the config does not loop', with_env(nil, nil, function(env)
  env.eval()
  local O = env.mod('overrides')
  local w = env.stub.new_window()
  env.stub.emit_reloaded(w)
  -- From here on every write re-evaluates and fires window-config-reloaded,
  -- and the stub raises if that ever recurses without end.
  env.stub.attach(function()
    env.eval()
  end)
  local evals = env.stub.evals

  O.set_channel(w, 'parallax', { vertical = 80 })
  T.eq(w.writes, 1)
  T.eq(env.stub.evals - evals, 1, 'exactly the one re-evaluation the write causes')

  -- The same value again, through the module instance the re-evaluation made.
  env.mod('overrides').set_channel(w, 'parallax', { vertical = 80 })
  T.eq(w.writes, 1, 'identical composition: no write')
  env.mod('overrides').set_channel(w, 'parallax', { vertical = 90 })
  T.eq(w.writes, 2)
  T.eq(env.stub.evals - evals, 2)
end))

---------------------------------------------------------------------------
-- Add-on precedence reaches the aggregator
---------------------------------------------------------------------------

T.test('add-on: a user-set background is never overridden, not even by parallax', with_env(nil, nil, function(env)
  local user_bg = { { source = { Color = '#123456' } } }
  local config = env.eval({ background = user_bg })
  T.eq(config.background, user_bg, 'the base config keeps the user background')
  local w = env.stub.new_window()
  env.stub.emit_reloaded(w)
  env.mod('overrides').set_channel(w, 'parallax', { vertical = 200 })
  T.eq(w.writes, 0)
  T.eq(w:get_config_overrides(), nil)
end))

T.test('add-on: a user-set colors table is never overridden by a preview', with_env(nil, nil, function(env)
  local _, handle = env.eval({ colors = { foreground = '#ffffff' } })
  local w = env.stub.new_window()
  env.stub.emit_reloaded(w)
  env.mod('overrides').preview_set(w, assert(handle.preview_payload('local:other')), {})
  local o = w:get_config_overrides() or {}
  T.eq(o.colors, nil, 'colors belongs to the user')
  T.ok(o.background ~= nil, 'keys the user did not set still preview')
end))

T.test('add-on: the aggregator itself refuses owned keys, whoever built the payload', with_env(nil, nil, function(env)
  -- preview_payload already resolves with the owned keys dropped, so that path
  -- never offers `colors`. A different producer (the TUI's OSC preview, a
  -- picker) might. Ownership must hold at the point of writing.
  env.eval({ colors = { foreground = '#ffffff' }, window_background_opacity = 0.5 })
  local w = env.stub.new_window()
  env.stub.emit_reloaded(w)
  env.mod('overrides').preview_set(w, {
    config = {
      colors = { foreground = '#000000' },
      window_background_opacity = 0.1,
      window_padding = { left = 3, right = 3, top = 3, bottom = 3 },
    },
  }, {})
  local o = w:get_config_overrides()
  T.eq(o.colors, nil, 'user-owned colors')
  T.eq(o.window_background_opacity, nil, 'user-owned opacity')
  T.eq(o.window_padding.left, 3, 'a key the user left alone still previews')
end))

T.test('replace mode: the same preview does override colors', with_env(nil, { mode = 'replace' }, function(env)
  local _, handle = env.eval({ colors = { foreground = '#ffffff' } })
  local w = env.stub.new_window()
  env.stub.emit_reloaded(w)
  env.mod('overrides').preview_set(w, assert(handle.preview_payload('local:other')), {})
  T.ok(w:get_config_overrides().colors ~= nil)
end))

---------------------------------------------------------------------------
-- Preview expiry through the real update-status handler
---------------------------------------------------------------------------

T.test('a TUI preview needs its heartbeat: it lapses on status ticks when the TUI dies', with_env(nil, nil, function(env)
  local _, handle = env.eval()
  local O = env.mod('overrides')
  local clock = 1000
  env.mod('platform')._clock = function()
    return clock
  end
  local w = env.stub.new_window()
  env.stub.emit_reloaded(w)

  O.preview_set(w, assert(handle.preview_payload('local:other')), { owner = 'tui', seq = 1 })
  T.ok(w:get_config_overrides().colors ~= nil, 'preview applied')

  clock = 1002
  env.stub.emit_status(w)
  T.ok(w:get_config_overrides().colors ~= nil, 'still within the expiry window')

  O.preview_set(w, nil, { owner = 'tui', seq = 2 }) -- heartbeat: renews, writes nothing
  clock = 1004
  env.stub.emit_status(w)
  T.ok(w:get_config_overrides().colors ~= nil, 'the heartbeat pushed the expiry out')

  clock = 1010 -- the TUI crashed: no more heartbeats
  env.stub.emit_status(w)
  T.eq((w:get_config_overrides() or {}).colors, nil, 'the window reverted by itself')
end))

T.test('a stale preview (older sequence number) is ignored', with_env(nil, nil, function(env)
  local _, handle = env.eval()
  local O = env.mod('overrides')
  local w = env.stub.new_window()
  env.stub.emit_reloaded(w)
  local other = assert(handle.preview_payload('local:other'))
  T.eq(O.preview_set(w, other, { owner = 'tui', seq = 5 }), true)
  local writes = w.writes
  T.eq(O.preview_set(w, assert(handle.preview_payload('local:mine')), { owner = 'tui', seq = 4 }), false)
  T.eq(w.writes, writes)
  T.ok(w:get_config_overrides().colors.tab_bar.active_tab.fg_color == '#fabd2f', 'the newer preview stands')
end))

T.test('closed windows are forgotten on the next status tick', with_env(nil, nil, function(env)
  env.eval()
  local O = env.mod('overrides')
  local keep, gone = env.stub.new_window(), env.stub.new_window()
  O.set_channel(keep, 'parallax', { vertical = 10 })
  O.set_channel(gone, 'parallax', { vertical = 10 })
  gone:close()
  env.stub.emit_status(keep)
  T.ok(O.get_channel(keep, 'parallax') ~= nil)
  T.eq(O.get_channel(gone, 'parallax'), nil, 'state for a closed window is cleared')
end))
