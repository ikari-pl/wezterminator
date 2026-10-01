-- End-to-end tests for the engine around the aggregator: plugin/init.lua,
-- data, state, apply and platform, driven through the wezterm stub.
--
-- Every evaluation goes through stub.eval, which gives the plugin a FRESH Lua
-- state (module locals and event handlers gone, wezterm.GLOBAL kept), the way
-- WezTerm re-evaluates a config. The built-in layer is the repo's real
-- presets/ and themes/, so these tests also prove the shipped files load.
--
-- The aggregator's own behaviour (channels, expiry, no reload loop) is covered
-- in overrides_test.lua.

local T = ...
local stub_lib = T.stub

local ROOT
do
  local p = io.popen('cd "' .. T.root .. '" && pwd')
  ROOT = p:read('*l')
  p:close()
end

local function jsonq(text)
  return T.json.decode(text)
end

--- A stub, a temp tree and the options that point the engine at it.
local function setup(stub_opts, plugin_opts)
  local stub = stub_lib.new(stub_opts)
  local tmp = stub.tmpdir()
  local dirs = {
    ['local'] = tmp .. '/config',
    fleet = tmp .. '/data/fleet',
    state = tmp .. '/state',
    data = tmp .. '/data',
  }
  local opts = { dir = ROOT, dirs = dirs, mode = 'add-on' }
  for k, v in pairs(plugin_opts or {}) do
    opts[k] = v
  end
  local env = { stub = stub, tmp = tmp, dirs = dirs, opts = opts }

  function env.eval(config)
    return stub.eval(ROOT, opts, config or {})
  end
  function env.write_state(active, history)
    stub.write(dirs.state .. '/state.json', string.format(
      '{"schema_version": 1, "active_preset": "%s", "history": %s}', active, history or '[]'))
  end
  function env.state_doc()
    local text = stub.read(dirs.state .. '/state.json')
    return text and jsonq(text), text
  end
  function env.module(name)
    return require('wzt.' .. name)
  end
  return env
end

local function cleanup(env)
  env.stub.cleanup()
end

--- Run a test body with automatic cleanup, even when it fails.
local function with_env(stub_opts, plugin_opts, fn)
  return function()
    local env = setup(stub_opts, plugin_opts)
    local ok, err = pcall(fn, env)
    cleanup(env)
    if not ok then
      error(err, 0)
    end
  end
end

---------------------------------------------------------------------------
-- Applying a preset to the config
---------------------------------------------------------------------------

T.test('with no state the default preset is applied (and nothing is written)', with_env(nil, nil, function(env)
  env.stub.track_io()
  local config, handle = env.eval()
  T.eq(handle.result.active.id, 'builtin:cpc-cool')
  T.eq(handle.result.notices[1].code, 'no_active_preset')
  T.eq(config.colors.tab_bar.active_tab.bg_color, '#000080', 'tab bar from the theme palette')
  T.eq(config.window_padding.left, 4)
  T.eq(config.window_background_opacity, 1)
  T.eq(config.font_size, 14)
  T.deep_eq(config.font.font_with_fallback[1], 'Terminess Nerd Font Mono')
  T.eq(#env.stub.io_writes, 0, 'config evaluation must not write files, got: ' .. T.show(env.stub.io_writes))
end))

T.test('Ember: stock scheme keeps the art base colour as its background', with_env(nil, nil, function(env)
  env.write_state('builtin:ember')
  local config = env.eval()
  T.eq(config.color_scheme, 'GruvboxDarkHard')
  T.eq(config.colors.background, '#0c0706', 'art base wins over the scheme background')
  T.eq(config.colors.tab_bar.active_tab.fg_color, '#fabd2f')
  T.eq(config.background[1].source.Color, '#0c0706', 'first background layer is the base colour')
end))

T.test('with no art on disk the theme fallback layers are used', with_env(nil, nil, function(env)
  local config, handle = env.eval()
  T.eq(handle.background.source, 'fallback')
  T.ok(#config.background >= 2, 'base colour plus at least one fallback layer')
  T.eq(config.background[2].source.Color ~= nil or config.background[2].source.Gradient ~= nil, true)
end))

T.test('motion.scrollback_parallax = false pins every layer', with_env(nil, nil, function(env)
  -- A local preset that is cpc-cool with parallax turned off.
  local text = T.read_file(ROOT .. '/presets/cpc-cool.json')
  text = text:gsub('"builtin:cpc%-cool"', '"local:still"', 1):gsub('"scrollback_parallax": true', '"scrollback_parallax": false')
  env.stub.write(env.dirs['local'] .. '/presets/still.json', text)
  env.write_state('local:still')
  local _, handle = env.eval()
  T.eq(handle.result.active.id, 'local:still')
  T.eq(handle.result.resolved.parts.motion.scrollback_parallax, false)
  -- No art on disk, so only fallback layers; assert via a stack that has files:
  local layers = handle.background.layers
  for i = 2, #layers do
    T.ok(layers[i].attachment == nil or layers[i].attachment == 'Fixed', 'layer ' .. i .. ' must not be parallax')
  end
end))

---------------------------------------------------------------------------
-- Add-on precedence (AE5) and platform limits
---------------------------------------------------------------------------

T.test('add-on mode never assigns a key the user set, and reports it overruled', with_env(nil, nil, function(env)
  env.write_state('builtin:soft-nebula')
  local config, handle = env.eval({ font = 'MY OWN FONT', font_size = 20, window_background_opacity = 0.5 })
  T.eq(config.font, 'MY OWN FONT')
  T.eq(config.font_size, 20)
  T.eq(config.window_background_opacity, 0.5)
  local by_path = {}
  for _, o in ipairs(handle.result.overruled) do
    by_path[o.path] = o.config_key
  end
  T.eq(by_path.font, 'font', 'font part reported once, under the first owned key')
  T.eq(by_path['chrome.opacity'], 'window_background_opacity')
  T.eq(config.window_padding.left, 8, 'fields the user left alone still apply')
end))

T.test('replace mode owns the whole config', with_env(nil, { mode = 'replace' }, function(env)
  env.write_state('builtin:soft-nebula')
  local config, handle = env.eval({ window_background_opacity = 0.5 })
  T.eq(config.window_background_opacity, 0.94)
  T.eq(#handle.result.overruled, 0)
end))

T.test('blur: set where the platform has a key, reported unavailable where it does not', function()
  local with = setup({ config_keys = { macos_window_background_blur = true } })
  with.write_state('builtin:soft-nebula')
  local config, handle = with.eval()
  T.eq(config.macos_window_background_blur, 22)
  T.eq(#handle.unavailable, 0)
  cleanup(with)

  local without = setup({ target_triple = 'x86_64-unknown-linux-gnu', config_keys = {} })
  without.write_state('builtin:soft-nebula')
  config, handle = without.eval()
  for _, k in ipairs({ 'macos_window_background_blur', 'kde_window_background_blur', 'wayland_window_background_blur' }) do
    T.eq(config[k], nil, k .. ' must not be set')
  end
  T.eq(handle.unavailable[1].path, 'chrome.blur')
  T.ok(handle.unavailable[1].reason ~= nil and handle.unavailable[1].reason ~= '')
  cleanup(without)
end)

T.test('blur on KDE/Wayland builds is a plain on/off key, not a radius', with_env(
  { target_triple = 'x86_64-unknown-linux-gnu', config_keys = { kde_window_background_blur = true } }, nil, function(env)
    env.write_state('builtin:soft-nebula')
    local config = env.eval()
    T.eq(config.kde_window_background_blur, true)
  end))

T.test('platform: OS, version and serde detection', function()
  local cases = {
    { 'aarch64-apple-darwin', 'macos' },
    { 'x86_64-apple-darwin', 'macos' },
    { 'x86_64-unknown-linux-gnu', 'linux' },
    { 'x86_64-pc-windows-msvc', 'windows' },
  }
  for _, c in ipairs(cases) do
    local stub = stub_lib.new({ target_triple = c[1] })
    stub.fresh_state()
    local platform = require 'wzt.platform'
    T.eq(platform.os(), c[2], c[1])
  end
  local stub = stub_lib.new({ version = '20230712-072601-f4abf8fd', serde = true })
  stub.fresh_state()
  local platform = require 'wzt.platform'
  T.eq(platform.version_date(), 20230712)
  T.eq(platform.meets_minimum(), false, 'older than 20240203')
  T.eq(platform.has_serde(), true)
  stub = stub_lib.new({ version = 'nightly-build', target_triple = 'x86_64-pc-windows-msvc' })
  stub.fresh_state()
  platform = require 'wzt.platform'
  T.eq(platform.meets_minimum(), true, 'unknown versions get the benefit of the doubt')
  local key, reason = platform.blur_key()
  T.eq(key, nil)
  T.ok(reason:find('Windows', 1, true))
end)

---------------------------------------------------------------------------
-- Comments never leak
---------------------------------------------------------------------------

local COMMENTED = [[
{
  "_": "A preset with comments at every depth.",
  "schema_version": 1,
  "id": "local:commented",
  "name": "Commented",
  "based_on": null,
  "_parts": "about parts",
  "parts": {
    "_": "parts note",
    "art": {"_": "art note", "theme": "builtin:cpc-cool"},
    "scheme": {"theme": "builtin:cpc-cool"},
    "palette": {"theme": "builtin:cpc-cool"},
    "font": {"_": "font note", "preferred": ["Menlo"], "fallback": ["Courier"]},
    "chrome": {
      "_": "chrome note",
      "_opacity": "about opacity",
      "opacity": 0.9,
      "padding": {"_": "pad note", "left": 1, "right": 1, "top": 1, "bottom": 1}
    },
    "status": {"style": "pill", "segments": ["clock"]},
    "motion": {"_": "motion note"}
  }
}
]]

T.test('a _ comment on a chrome object never reaches the config or a preview', with_env(nil, nil, function(env)
  env.stub.write(env.dirs['local'] .. '/presets/commented.json', COMMENTED)
  env.write_state('local:commented')
  local config, handle = env.eval()
  T.eq(handle.result.active.id, 'local:commented')
  T.eq(config.window_background_opacity, 0.9)
  T.eq(config.window_padding.left, 1)
  local hit, key = T.has_comment_key(config)
  T.ok(not hit, 'config contains comment key ' .. tostring(key))
  T.ok(not T.has_comment_key(handle.result), 'resolution output is comment-free')

  local payload = assert(handle.preview_payload('local:commented'))
  T.eq(payload.config.window_background_opacity, 0.9)
  hit, key = T.has_comment_key(payload)
  T.ok(not hit, 'preview payload contains comment key ' .. tostring(key))
end))

T.test('preview_payload refuses a preset that does not resolve', with_env(nil, nil, function(env)
  local _, handle = env.eval()
  local payload, why = handle.preview_payload('local:nope')
  T.eq(payload, nil)
  T.ok(why:find('local:nope', 1, true))
end))

---------------------------------------------------------------------------
-- Commit, undo, cycle (AE1)
---------------------------------------------------------------------------

local CPC, EMBER, NEBULA = 'builtin:cpc-cool', 'builtin:ember', 'builtin:soft-nebula'

T.test('AE1: a commit writes state.json and the next evaluation resolves it', with_env(nil, nil, function(env)
  local _, first = env.eval()
  T.eq(first.result.active.id, CPC, 'default before any commit')

  local state = env.module('state')
  T.eq(state.commit(env.dirs, EMBER), true)
  local doc = env.state_doc()
  T.eq(doc.active_preset, EMBER)

  -- "Simulated reload": a fresh Lua state reads the file again.
  local config, second = env.eval()
  T.eq(second.result.active.id, EMBER)
  T.eq(second.generation_changed, true, 'the commit changed the fingerprint')
  T.eq(config.color_scheme, 'GruvboxDarkHard')

  -- And it survives yet another fresh evaluation without any rewrite.
  env.stub.track_io()
  local _, third = env.eval()
  T.eq(third.result.active.id, EMBER)
  T.eq(third.generation_changed, false)
  T.eq(#env.stub.io_writes, 0)
end))

T.test('committing the already-active preset writes nothing', with_env(nil, nil, function(env)
  env.write_state(EMBER)
  env.eval()
  env.stub.track_io()
  local state = env.module('state')
  local ok, why = state.commit(env.dirs, EMBER)
  T.eq(ok, true)
  T.eq(why, 'unchanged')
  T.eq(#env.stub.io_writes, 0)
end))

T.test('undo: three commits then two undos lands on the first preset', with_env(nil, nil, function(env)
  env.eval()
  local state = env.module('state')
  for _, id in ipairs({ CPC, EMBER, NEBULA }) do
    T.eq(state.commit(env.dirs, id), true)
  end
  T.eq(env.state_doc().active_preset, NEBULA)
  T.eq(state.undo(env.dirs), EMBER)
  T.eq(state.undo(env.dirs), CPC)
  T.eq(env.state_doc().active_preset, CPC)
  local id, why = state.undo(env.dirs)
  T.eq(id, nil)
  T.eq(why, 'nothing to undo')
  -- The empty history is still an array on disk, not an object.
  local _, text = env.state_doc()
  T.ok(text:find('"history": []', 1, true), 'history must stay a JSON array: ' .. text)
end))

T.test('history stops at 20 entries, oldest dropped first', with_env(nil, nil, function(env)
  env.eval()
  local state = env.module('state')
  local ids = { CPC, EMBER, NEBULA }
  for i = 1, 25 do
    T.eq(state.commit(env.dirs, ids[(i - 1) % 3 + 1]), true)
  end
  local doc = env.state_doc()
  T.eq(#doc.history, 20)
  T.eq(doc.active_preset, ids[(25 - 1) % 3 + 1])
  -- History holds the OUTGOING preset of commits 2..25, i.e. ids of commits 1..24;
  -- the newest 20 are commits 5..24.
  T.eq(doc.history[1].preset, ids[(5 - 1) % 3 + 1])
  T.eq(doc.history[20].preset, ids[(24 - 1) % 3 + 1])
  T.ok(doc.history[1].at:match('^%d%d%d%d%-%d%d%-%d%dT%d%d:%d%d:%d%dZ$'), 'RFC 3339 timestamp: ' .. tostring(doc.history[1].at))
end))

T.test('commit keeps hand-written comments and unrelated fields', with_env(nil, nil, function(env)
  env.stub.write(env.dirs.state .. '/state.json', [[
{"_": "my notes", "schema_version": 1, "active_preset": "builtin:cpc-cool", "history": [],
 "install_mode": "add-on", "_install_mode": "chosen at setup"}]])
  env.eval()
  local state = env.module('state')
  T.eq(state.commit(env.dirs, EMBER), true)
  local doc = env.state_doc()
  T.eq(doc._, 'my notes')
  T.eq(doc._install_mode, 'chosen at setup')
  T.eq(doc.install_mode, 'add-on')
  T.eq(doc.history[1].preset, CPC)
end))

T.test('commit refuses to overwrite a corrupt or newer-schema state file', with_env(nil, nil, function(env)
  env.eval()
  local state = env.module('state')
  env.stub.write(env.dirs.state .. '/state.json', '{ this is not json')
  local ok, why = state.commit(env.dirs, EMBER)
  T.eq(ok, false)
  T.ok(tostring(why):find('corrupt', 1, true), tostring(why))
  T.eq(env.stub.read(env.dirs.state .. '/state.json'), '{ this is not json', 'file untouched')

  env.stub.write(env.dirs.state .. '/state.json', '{"schema_version": 2, "active_preset": "builtin:x", "history": []}')
  ok, why = state.commit(env.dirs, EMBER)
  T.eq(ok, false)
  T.ok(tostring(why):find('schema_version', 1, true), tostring(why))
end))

T.test('cycle steps through the catalog and wraps around', with_env(nil, nil, function(env)
  env.write_state(CPC)
  env.eval()
  local state = env.module('state')
  local ids = { CPC, EMBER, NEBULA }
  T.eq(state.cycle(env.dirs, 1, ids, CPC), EMBER)
  T.eq(state.cycle(env.dirs, 1, ids, EMBER), NEBULA)
  T.eq(state.cycle(env.dirs, 1, ids, NEBULA), CPC, 'wraps forward')
  T.eq(state.cycle(env.dirs, -1, ids, CPC), NEBULA, 'wraps backward')
end))

---------------------------------------------------------------------------
-- Fallback, toast, loading
---------------------------------------------------------------------------

T.test('a missing active preset falls back to the default and records one toast', with_env(nil, nil, function(env)
  env.write_state('local:gone')
  local _, handle = env.eval()
  T.eq(handle.result.active.id, CPC)
  T.eq(handle.result.active.fell_back, true)
  T.eq(handle.result.notices[1].code, 'active_preset_missing')

  local window = env.stub.new_window()
  env.stub.emit_reloaded(window)
  T.eq(#env.stub.toasts, 1)
  T.ok(env.stub.toasts[1].message:find('local:gone', 1, true), env.stub.toasts[1].message)
  env.stub.emit_reloaded(window)
  T.eq(#env.stub.toasts, 1, 'once per generation, not on every reload event')
end))

T.test('the layer cache holds across fresh states and a file edit starts a new generation', with_env(nil, nil, function(env)
  local _, a = env.eval()
  T.eq(a.generation_changed, true, 'first evaluation populates the cache')
  local _, b = env.eval()
  T.eq(b.generation_changed, false, 'unchanged files reuse the cache held in wezterm.GLOBAL')
  T.eq(b.generation, a.generation)

  env.stub.write(env.dirs['local'] .. '/presets/commented.json', COMMENTED)
  local _, c = env.eval()
  T.eq(c.generation_changed, true)
  T.eq(c.generation, a.generation + 1)
  local found = false
  for _, e in ipairs(c.result.catalog) do
    found = found or e.id == 'local:commented'
  end
  T.ok(found, 'the new local preset is in the catalog')
end))

T.test('a malformed file is skipped and reported, and the rest still loads', with_env(nil, nil, function(env)
  env.stub.write(env.dirs['local'] .. '/presets/broken.json', '{ not json')
  local _, handle = env.eval()
  T.eq(handle.result.active.id, CPC)
  T.eq(#handle.load_errors, 1)
  T.ok(handle.load_errors[1].path:find('broken.json', 1, true))
  local warned = false
  for _, l in ipairs(env.stub.logs) do
    warned = warned or (l.level == 'warn' and l.msg:find('broken.json', 1, true) ~= nil)
  end
  T.ok(warned, 'a warning names the broken file')
end))

T.test('layers merge local over fleet over built-in (overrides.json)', with_env(nil, nil, function(env)
  env.stub.write(env.dirs.fleet .. '/overrides.json',
    '{"schema_version": 1, "parts": {"chrome": {"opacity": 0.8, "padding": {"left": 9, "right": 9, "top": 9, "bottom": 9}}}}')
  env.stub.write(env.dirs['local'] .. '/overrides.json',
    '{"schema_version": 1, "_": "mine", "parts": {"chrome": {"opacity": 0.7}}}')
  local config = env.eval()
  T.eq(config.window_background_opacity, 0.7, 'local wins')
  T.eq(config.window_padding.left, 9, 'fleet value survives where local is silent')
end))

---------------------------------------------------------------------------
-- Files the engine writes, and when
---------------------------------------------------------------------------

local STUDIO = {
  { name = 'Built-in', width = 3024, height = 1964 },
  { name = 'Studio', width = 6016, height = 3384 },
}

T.test('first update-status records screens (unwatched) and the engine (state.json)', with_env({ screens = STUDIO }, nil, function(env)
  env.eval()
  local window = env.stub.new_window()
  env.stub.emit_status(window)

  local screens = jsonq(env.stub.read(env.dirs.state .. '/screens.json'))
  T.eq(screens.schema_version, 1)
  T.eq(#screens.screens, 2)
  T.eq(screens.screens[1].name, 'Built-in')
  T.eq(screens.screens[2].width, 6016)

  local doc = env.state_doc()
  T.eq(doc.engine.plugin_dir, ROOT)
  T.eq(doc.engine.version, '0.1.0')
  T.eq(doc.engine.schema_version, 1)
  T.eq(doc.active_preset, CPC, 'the schema requires an active preset next to the engine record')
  T.eq(env.stub.reloads, 1, 'a state.json that did not exist cannot have been watched, so reload explicitly')
end))

T.test('the engine record is written once and then left alone', with_env({ screens = STUDIO }, nil, function(env)
  env.eval()
  local state = env.module('state')
  local info = { plugin_dir = ROOT, version = '0.1.0', schema_version = 1, default_preset = CPC }
  T.eq(state.record_engine(env.dirs, info), true)
  env.stub.track_io()
  T.eq(state.record_engine(env.dirs, info), false, 'identical record: no write, so no reload loop')
  T.eq(#env.stub.io_writes, 0)
  info.version = '0.2.0'
  T.eq(state.record_engine(env.dirs, info), true, 'a new engine version is recorded')
  T.eq(env.state_doc().engine.version, '0.2.0')
end))

T.test('recording the engine keeps the active preset, history and comments', with_env(nil, nil, function(env)
  env.stub.write(env.dirs.state .. '/state.json',
    '{"_": "keep me", "schema_version": 1, "active_preset": "builtin:ember", "history": [{"preset": "builtin:cpc-cool", "at": "2026-10-01T10:00:00Z"}]}')
  env.eval()
  local state = env.module('state')
  T.eq(state.record_engine(env.dirs, { plugin_dir = ROOT, version = '0.1.0', schema_version = 1, default_preset = CPC }), true)
  local doc = env.state_doc()
  T.eq(doc._, 'keep me')
  T.eq(doc.active_preset, EMBER)
  T.eq(#doc.history, 1)
  T.eq(doc.engine.plugin_dir, ROOT)
  T.eq(env.stub.reloads, 0, 'an existing watched file reloads by itself')
end))

T.test('an unchanged screen list writes no file; a changed one does', with_env({ screens = STUDIO }, nil, function(env)
  env.eval()
  local state = env.module('state')
  local list = state.current_screens()
  env.stub.track_io()
  T.eq(state.write_screens(env.dirs, list), true, 'first record')
  T.ok(env.stub.writes_matching('screens%.json') >= 1)
  env.stub.reset_io()
  T.eq(state.write_screens(env.dirs, list), false, 'same list')
  T.eq(state.write_screens(env.dirs, state.current_screens()), false, 'same list, read again')
  T.eq(#env.stub.io_writes, 0)

  env.stub.screens = { { name = 'Studio', width = 6016, height = 3384 } }
  T.eq(state.write_screens(env.dirs, state.current_screens()), true, 'screen unplugged')
  T.eq(#jsonq(env.stub.read(env.dirs.state .. '/screens.json')).screens, 1)
end))

T.test('inside the mux server (no wezterm.gui) nothing is recorded', with_env({ gui = false, screens = STUDIO }, nil, function(env)
  env.eval()
  env.stub.track_io()
  local window = env.stub.new_window()
  env.stub.emit_status(window)
  T.eq(#env.stub.io_writes, 0)
  T.eq(env.stub.exists(env.dirs.state .. '/screens.json'), false)
end))

T.test('the watch list names state.json and layer files, never screens.json or the state directory', with_env(nil, nil, function(env)
  env.stub.write(env.dirs['local'] .. '/presets/commented.json', COMMENTED)
  env.eval()
  local seen = {}
  for _, p in ipairs(env.stub.watched) do
    seen[p] = true
    T.ok(not p:find('screens.json', 1, true), 'watched: ' .. p)
    T.ok(p ~= env.dirs.state, 'the state directory itself must not be watched')
  end
  T.ok(seen[env.dirs.state .. '/state.json'], 'state.json is watched')
  T.ok(seen[env.dirs['local'] .. '/presets/commented.json'], 'local preset files are watched')
  T.ok(seen[env.dirs['local'] .. '/overrides.json'], 'overrides.json is watched even before it exists')
end))

---------------------------------------------------------------------------
-- Art lookup order: dev path, user art, shipped art, fallback (R15, U4)
---------------------------------------------------------------------------

local function mine_preset()
  return (COMMENTED:gsub('"local:commented"', '"local:mine"'):gsub('"art": {"_": "art note", "theme": "builtin:cpc%-cool"}',
    '"art": {"theme": "local:mine"}'))
end

local function mine_theme()
  local text, n = T.read_file(ROOT .. '/themes/cpc-cool/theme.json'):gsub('"id": "builtin:cpc%-cool"', '"id": "local:mine"')
  T.eq(n, 1, 'theme id substituted exactly once')
  return text
end

local function touch_art(env, dir)
  for _, id in ipairs({ 'nebula', 'stars', 'grid', 'haze' }) do
    env.stub.write(dir .. '/' .. id .. '.png', '')
  end
end

T.test('art is looked up dev, then user, then shipped, then fallback', with_env(nil, nil, function(env)
  local L = env.dirs['local']
  env.stub.write(L .. '/presets/mine.json', mine_preset())
  env.stub.write(L .. '/themes/mine/theme.json', mine_theme())
  env.stub.write(env.dirs.state .. '/screens.json', '{"schema_version": 1, "screens": [{"width": 1000, "height": 500}]}')
  env.write_state('local:mine')

  local _, h = env.eval()
  T.eq(h.result.active.id, 'local:mine')
  T.eq(h.background.source, 'fallback', 'no art anywhere')

  local shipped = L .. '/themes/mine/art/1000x500'
  touch_art(env, shipped)
  _, h = env.eval()
  T.eq(h.background.source, 'shipped')
  T.eq(h.background.layers[2].source.File, shipped .. '/nebula.png')
  T.eq(h.background.layers[2].width, 1000, 'pinned to the art native size')
  T.eq(h.background.layers[2].height, 500)

  local user = env.dirs.data .. '/art/mine/1000x500'
  touch_art(env, user)
  _, h = env.eval()
  T.eq(h.background.source, 'user', 'user art beats shipped art')
  T.eq(h.background.layers[2].source.File, user .. '/nebula.png')

  os.remove(user .. '/grid.png')
  _, h = env.eval()
  T.eq(h.background.source, 'shipped', 'an incomplete user set is not used')

  local dev = env.tmp .. '/dev'
  touch_art(env, dev .. '/mine')
  env.stub.write(L .. '/machine.json', '{"schema_version": 1, "dev_art_path": "' .. dev .. '"}')
  _, h = env.eval()
  T.eq(h.background.source, 'dev', 'the development path is consulted first')
  T.eq(h.background.layers[2].source.File, dev .. '/mine/nebula.png')
end))

T.test('art at another resolution is not used', with_env(nil, nil, function(env)
  local L = env.dirs['local']
  env.stub.write(L .. '/presets/mine.json', mine_preset())
  env.stub.write(L .. '/themes/mine/theme.json', mine_theme())
  env.stub.write(env.dirs.state .. '/screens.json', '{"schema_version": 1, "screens": [{"width": 1000, "height": 500}]}')
  env.write_state('local:mine')
  touch_art(env, L .. '/themes/mine/art/2000x1000')
  local _, h = env.eval()
  T.eq(h.background.source, 'fallback')
end))

T.test('the largest connected screen picks the art resolution', function()
  local apply
  local stub = stub_lib.new()
  stub.fresh_state()
  apply = require 'wzt.apply'
  local res = apply.pick_resolution({
    { name = 'a', width = 3024, height = 1964 },
    { name = 'b', width = 6016, height = 3384 },
  }, nil)
  T.eq(res.w, 6016)
  T.eq(res.h, 3384)
  -- screen_overrides stand in when WezTerm reports nothing.
  res = apply.pick_resolution(nil, { { width = 5120, height = 2880 } })
  T.eq(res.w, 5120)
  T.eq(apply.pick_resolution(nil, nil), nil)
end)

---------------------------------------------------------------------------
-- Data encoding
---------------------------------------------------------------------------

T.test('encode: stable key order, empty arrays stay arrays, round trip', function()
  local stub = stub_lib.new()
  stub.fresh_state()
  local data = require 'wzt.data'
  local resolve = require 'wzt.resolve'
  local doc = {
    active_preset = 'builtin:x',
    history = resolve.new_array(),
    schema_version = 1,
    note = 'quote " backslash \\ newline \n tab \t unicode é',
    nested = { b = 2, a = 1, list = { 1, 2, 3 } },
  }
  local text = data.encode(doc)
  T.ok(text:find('^{\n  "schema_version": 1,'), 'schema_version leads: ' .. text)
  T.ok(text:find('"history": []', 1, true))
  local back = data.decode(text)
  T.eq(back.note, doc.note)
  T.eq(back.nested.list[3], 3)
  T.eq(data.encode(back), text, 'encoding is deterministic')
end)

