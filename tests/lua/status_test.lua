-- Status bar (U3): sparklines, segment visibility with and without the stats
-- cache, both styles, history that survives a fresh Lua state, and the
-- once-per-second collector rule.
--
-- Keys, parallax, pickers and the rest of U3 are in keys_test.lua and
-- features_test.lua.

local T = ...
local stub_lib = T.stub
local common = require 'wzt.segments.common'

local function utf8_len(s)
  local n = 0
  for _ in s:gmatch('[%z\1-\127\194-\244][\128-\191]*') do
    n = n + 1
  end
  return n
end

local function forget_modules()
  for name in pairs(package.loaded) do
    if name:match('^wzt%.') and name ~= 'wzt.segments.common' then
      package.loaded[name] = nil
    end
  end
end

--- A stub wezterm with the extras the status bar uses, modules reloaded
--- against it, and a controllable clock.
local function with_status(fn, extra)
  local stub = stub_lib.new()
  local w = stub.wezterm
  w.format = function(elements)
    return elements -- tests read the elements themselves
  end
  w.strftime = function()
    return '12:34'
  end
  w.battery_info = function()
    return { { state_of_charge = 0.8, state = 'Discharging' } }
  end
  w.background_child_process = function(args)
    stub.spawned = stub.spawned or {}
    stub.spawned[#stub.spawned + 1] = args
  end
  if extra then
    extra(stub)
  end
  package.loaded.wezterm = w
  forget_modules()
  local platform = require 'wzt.platform'
  local clock = 1000
  platform._clock = function()
    return clock
  end
  local env = {
    stub = stub,
    status = require 'wzt.status',
    platform = platform,
    tick = function(n)
      clock = clock + (n or 1)
    end,
  }
  local ok, err = pcall(fn, env)
  stub.cleanup()
  forget_modules()
  package.loaded.wezterm = nil
  if not ok then
    error(err, 0)
  end
end

local function make_window(stub)
  local win = stub.new_window()
  function win:set_left_status(s)
    self.left = s
  end
  function win:set_right_status(s)
    self.right = s
  end
  function win:active_workspace()
    return 'default'
  end
  function win:leader_is_active()
    return false
  end
  return win
end

local function make_pane(cwd, vars)
  return {
    get_current_working_dir = function()
      return cwd and { file_path = cwd } or nil
    end,
    get_user_vars = function()
      return vars or {}
    end,
  }
end

local ALL = {
  'load', 'memory', 'memory_pressure', 'tailscale', 'warp', 'tunnels', 'aws_vpn',
  'cwd', 'font', 'battery', 'exit_code', 'clock', 'workspace',
}

local UI = {
  bg = '#000001', surface = '#000002', fg = '#000003', fg_dim = '#000004',
  accent = '#000005', accent_alt = '#000006', ok = '#000007', warn = '#000008',
  bad = '#000009', info = '#00000a',
}

local function ids_of(list)
  return table.concat(list, ',')
end

---------------------------------------------------------------------------
-- Sparklines
---------------------------------------------------------------------------

T.test('sparkline: a flat series renders mid-height at fixed width', function()
  local buf = {}
  for _ = 1, common.SAMPLES do
    common.push(buf, 0.5, common.SAMPLES)
  end
  local spark = common.sparkline(buf, common.SAMPLES)
  T.eq(utf8_len(spark), common.SAMPLES)
  -- Mid-height is one of the two middle blocks, never the floor or the ceiling.
  local mids = { [common.BLOCKS[4]] = true, [common.BLOCKS[5]] = true }
  local seen = {}
  for ch in spark:gmatch('[\226][\150-\151][\128-\191]') do
    seen[ch] = true
  end
  local distinct = 0
  for ch in pairs(seen) do
    distinct = distinct + 1
    T.ok(mids[ch], 'flat series uses a middle block, got ' .. ch)
  end
  T.eq(distinct, 1, 'one glyph repeated')
end)

T.test('sparkline: a partially filled buffer pads to the fixed width', function()
  local buf = {}
  common.push(buf, 1.0, common.SAMPLES)
  common.push(buf, 0.0, common.SAMPLES)
  local spark = common.sparkline(buf, common.SAMPLES)
  T.eq(utf8_len(spark), common.SAMPLES)
  T.eq(spark:sub(1, common.SAMPLES - 2), string.rep(' ', common.SAMPLES - 2), 'padding on the left')
  T.eq(utf8_len(common.sparkline({}, common.SAMPLES)), common.SAMPLES, 'an empty buffer is all padding')
end)

T.test('sparkline: autoscaled to the window, extremes reach the first and last block', function()
  local spark = common.sparkline({ 1, 5, 9 }, 3)
  T.eq(spark, common.BLOCKS[1] .. common.BLOCKS[5] .. common.BLOCKS[8],
    'minimum is the lowest block, maximum the full block, the midpoint between')
end)

T.test('sparkline: only the newest `width` samples are drawn', function()
  local buf = {}
  for i = 1, 40 do
    buf[i] = i
  end
  T.eq(utf8_len(common.sparkline(buf, 16)), 16)
end)

T.test('push keeps a ring buffer to its cap', function()
  local buf = {}
  for i = 1, 30 do
    common.push(buf, i, 16)
  end
  T.eq(#buf, 16)
  T.eq(buf[1], 15)
  T.eq(buf[16], 30)
end)

---------------------------------------------------------------------------
-- The stats cache
---------------------------------------------------------------------------

T.test('parse_line reads key=value pairs and rejects text without any', function()
  with_status(function(env)
    local t = env.status.parse_line('load=1.5 memused=40.0 ts=up aws_vpn=off')
    T.eq(t.load, '1.5')
    T.eq(t.ts, 'up')
    T.eq(t.aws_vpn, 'off')
    T.eq(env.status.parse_line('nothing here'), nil)
    T.eq(env.status.parse_line(nil), nil)
  end)
end)

T.test('read_stats: absent file is nil, present file is its first line', function()
  with_status(function(env)
    local dir = env.stub.tmpdir()
    T.eq(env.status.read_stats(dir .. '/stats'), nil)
    env.stub.write(dir .. '/stats', 'load=2.5 ncpu=8\nignored=1\n')
    local t = env.status.read_stats(dir .. '/stats')
    T.eq(t.load, '2.5')
    T.eq(t.ignored, nil, 'only the first line')
  end)
end)

---------------------------------------------------------------------------
-- Segment visibility
---------------------------------------------------------------------------

T.test('AE7: a cache line lacking warp hides WARP; Tailscale renders from ts=up', function()
  with_status(function(env)
    local shown = env.status.collect({ 'tailscale', 'warp' }, { stats = { ts = 'up' }, history = {} })
    T.eq(#shown, 1)
    T.eq(shown[1].id, 'tailscale')
    T.eq(shown[1].item.text, 'TS')
    T.eq(shown[1].item.tone, 'ok')

    local both = env.status.collect({ 'tailscale', 'warp' }, { stats = { ts = 'off', warp = 'wait' }, history = {} })
    T.eq(#both, 2)
    T.eq(both[1].item.tone, 'fg_dim', 'a probe that ran and found nothing is dimmed, not hidden')
    T.eq(both[2].item.tone, 'warn')
  end)
end)

T.test('with a cache: stats segments render, and a missing key hides only its own segment', function()
  with_status(function(env)
    local stats = { load = '2.00', ncpu = '8', memused = '40', memtotal = '128', pressure = '1', ts = 'up', utun = '1' }
    local shown = env.status.collect(ALL, {
      stats = stats, history = { load = { 1, 2 }, mem = { 3, 4 } },
      font = 'FiraCode Nerd Font Mono', now = 1000,
      window = make_window(env.stub), pane = make_pane('/tmp'),
    })
    local ids = {}
    for _, s in ipairs(shown) do
      ids[#ids + 1] = s.id
    end
    -- No warp, no aws_vpn in the cache line; exit_code needs the shell to report.
    T.eq(ids_of(ids), 'load,memory,memory_pressure,tailscale,tunnels,cwd,font,battery,clock,workspace')
  end)
end)

T.test('with no cache file only the Lua-native segments render, battery included', function()
  with_status(function(env)
    local shown = env.status.collect(ALL, {
      stats = nil, history = {}, font = 'Menlo', now = 1000,
      window = make_window(env.stub), pane = make_pane('/tmp', { wezterm_exit_code = '0' }),
    })
    local ids = {}
    for _, s in ipairs(shown) do
      ids[#ids + 1] = s.id
    end
    T.eq(ids_of(ids), 'cwd,font,battery,exit_code,clock,workspace')
  end)
end)

T.test('battery is Lua-native only: a battery key in the cache is ignored', function()
  with_status(function(env)
    env.stub.wezterm.battery_info = function()
      return {}
    end
    local shown = env.status.collect({ 'battery' }, { stats = { battery = '55' }, history = {} })
    T.eq(#shown, 0, 'no battery hardware means no segment, whatever the cache says')

    env.stub.wezterm.battery_info = function()
      return { { state_of_charge = 0.15, state = 'Charging' } }
    end
    local low = env.status.collect({ 'battery' }, { stats = nil, history = {} })
    T.eq(#low, 1)
    T.eq(low[1].item.text, '15%')
    T.eq(low[1].item.tone, 'bad')
  end)
end)

T.test('exit_code and workspace tones follow their data', function()
  with_status(function(env)
    local win = make_window(env.stub)
    local ctx = { history = {}, window = win, pane = make_pane('/tmp', { wezterm_exit_code = '2' }) }
    local shown = env.status.collect({ 'exit_code', 'workspace' }, ctx)
    T.eq(shown[1].item.tone, 'bad')
    T.eq(shown[2].side, 'left', 'workspace lives in the left status area')
    T.eq(shown[2].item.tone, 'fg_dim', 'the default workspace is dimmed')
  end)
end)

T.test('an unknown segment id is ignored and a failing segment does not take the bar down', function()
  with_status(function(env)
    package.loaded['wzt.segments.clock'] = { id = 'clock', side = 'right', render = function()
      error('boom')
    end }
    local shown = env.status.collect({ 'nonsense', 'clock', 'battery' }, { history = {} })
    T.eq(#shown, 1)
    T.eq(shown[1].id, 'battery')
  end)
end)

T.test('load and memory colour by utilisation, memory pressure by level', function()
  with_status(function(env)
    local function tone(id, stats)
      return env.status.collect({ id }, { stats = stats, history = {} })[1].item.tone
    end
    T.eq(tone('load', { load = '1', ncpu = '8' }), 'accent')
    T.eq(tone('load', { load = '5', ncpu = '8' }), 'warn')
    T.eq(tone('load', { load = '8', ncpu = '8' }), 'bad')
    T.eq(tone('memory', { memused = '120', memtotal = '128' }), 'bad')
    T.eq(tone('memory_pressure', { pressure = '1' }), 'ok')
    T.eq(tone('memory_pressure', { pressure = '2' }), 'warn')
    T.eq(tone('memory_pressure', { pressure = '4' }), 'bad')
    T.eq(env.status.collect({ 'memory_pressure' }, { stats = { pressure = 'na' }, history = {} })[1], nil,
      'pressure=na (no PSI) hides the segment')
  end)
end)

---------------------------------------------------------------------------
-- Styles
---------------------------------------------------------------------------

local function texts(elements)
  local out = {}
  for _, e in ipairs(elements) do
    if type(e) == 'table' and e.Text then
      out[#out + 1] = e.Text
    end
  end
  return table.concat(out)
end

local function colours(elements)
  local out = {}
  for _, e in ipairs(elements) do
    if type(e) == 'table' then
      local c = (e.Foreground and e.Foreground.Color) or (e.Background and e.Background.Color)
      if c then
        out[#out + 1] = c
      end
    end
  end
  return out
end

T.test('sparkline style: sparkline glyphs, thin separators, colours from the palette only', function()
  with_status(function(env)
    local stats = { load = '2.00', ncpu = '8', memused = '40', memtotal = '128', pressure = '1' }
    local shown = env.status.collect({ 'load', 'memory' }, {
      stats = stats, history = { load = { 1, 2, 3 }, mem = { 4, 5, 6 } },
    })
    local els = env.status.elements(shown, 'right', 'sparkline', UI)
    local text = texts(els)
    T.ok(text:find('│', 1, true), 'a separator between segments')
    T.ok(text:find(common.BLOCKS[8], 1, true), 'a sparkline is drawn')
    T.ok(text:find('2.00', 1, true) and text:find('40/128G', 1, true))
    local allowed = {}
    for _, hex in pairs(UI) do
      allowed[hex] = true
    end
    for _, c in ipairs(colours(els)) do
      T.ok(allowed[c], 'colour ' .. c .. ' comes from palette.ui')
    end
    for _, e in ipairs(els) do
      T.ok(e ~= 'ResetAttributes', 'no pill resets in the sparkline style')
    end
  end)
end)

T.test('pill style: surface background per pill, no sparklines, a reset between pills', function()
  with_status(function(env)
    local stats = { load = '2.00', ncpu = '8', memused = '40', memtotal = '128' }
    local shown = env.status.collect({ 'load', 'memory' }, {
      stats = stats, history = { load = { 1, 2, 3 }, mem = { 4, 5, 6 } },
    })
    local els = env.status.elements(shown, 'right', 'pill', UI)
    local backgrounds, resets = 0, 0
    for _, e in ipairs(els) do
      if e == 'ResetAttributes' then
        resets = resets + 1
      elseif type(e) == 'table' and e.Background then
        backgrounds = backgrounds + 1
        T.eq(e.Background.Color, UI.surface)
      end
    end
    T.eq(backgrounds, 2, 'one filled pill per segment')
    T.ok(resets >= 2, 'pills are separated and the last one is closed')
    for block = 1, #common.BLOCKS do
      T.ok(not texts(els):find(common.BLOCKS[block], 1, true), 'no sparkline glyphs in pill style')
    end
  end)
end)

T.test('a palette without a semantic key falls back to a related key, never to hex', function()
  with_status(function(env)
    local ui = { fg = '#111111', accent = '#222222' }
    T.eq(env.status.colour(ui, 'info'), '#222222', 'info falls back to accent')
    T.eq(env.status.colour(ui, 'fg_dim'), '#111111', 'fg_dim falls back to fg')
    T.eq(env.status.colour(ui, 'ok'), nil, 'no related key: no colour, not an invented one')
    T.eq(env.status.colour(nil, 'ok'), nil)
  end)
end)

---------------------------------------------------------------------------
-- update-status
---------------------------------------------------------------------------

local function status_opts(env, dir, segments)
  return {
    status = { style = 'sparkline', segments = segments or ALL },
    ui = UI,
    font = 'Menlo',
    stats_path = dir .. '/stats',
    home = '/Users/x',
  }
end

T.test('update: no cache file renders the native segments and sets both areas', function()
  with_status(function(env)
    local dir = env.stub.tmpdir()
    local win = make_window(env.stub)
    local out = env.status.update(win, make_pane('/tmp'), status_opts(env, dir))
    T.eq(ids_of(out.shown), 'cwd,font,battery,clock,workspace')
    T.ok(type(win.left) == 'table' and #win.left > 0, 'workspace goes to the left status')
    T.ok(type(win.right) == 'table' and #win.right > 0)
  end)
end)

T.test('update: history is sampled once per second in total, not per window, and lives in GLOBAL', function()
  with_status(function(env)
    local dir = env.stub.tmpdir()
    local opts = status_opts(env, dir, { 'load' })
    env.stub.write(dir .. '/stats', 'load=1.0 ncpu=4\n')
    local a, b = make_window(env.stub), make_window(env.stub)
    env.status.update(a, make_pane('/tmp'), opts)
    env.status.update(b, make_pane('/tmp'), opts)
    T.eq(#env.platform.store_get('wzt_status').load, 1, 'two windows in one second: one sample')

    env.tick()
    env.stub.write(dir .. '/stats', 'load=2.0 ncpu=4\n')
    env.status.update(a, make_pane('/tmp'), opts)
    T.eq(#env.platform.store_get('wzt_status').load, 2)

    -- A fresh Lua state (as after any set_config_overrides) keeps the history.
    env.stub.fresh_state()
    forget_modules()
    package.loaded.wezterm = env.stub.wezterm
    local platform2 = require 'wzt.platform'
    local clock = 1001
    platform2._clock = function()
      return clock
    end
    T.eq(#platform2.store_get('wzt_status').load, 2, 'sparkline history survives a fresh state')
  end)
end)

T.test('collector: started at most once per second across windows, never when disabled', function()
  with_status(function(env)
    local dir = env.stub.tmpdir()
    local opts = status_opts(env, dir, { 'clock' })
    opts.binary = '/opt/wzt/wezterminator'
    local a, b = make_window(env.stub), make_window(env.stub)
    env.status.update(a, make_pane('/tmp'), opts)
    env.status.update(b, make_pane('/tmp'), opts)
    T.eq(#env.stub.spawned, 1, 'two windows, one tick, one process')
    T.eq(env.stub.spawned[1][1], '/opt/wzt/wezterminator')
    T.eq(env.stub.spawned[1][2], 'stats')

    env.tick()
    env.status.update(a, make_pane('/tmp'), opts)
    T.eq(#env.stub.spawned, 2)

    env.tick()
    opts.collect = false
    env.status.update(a, make_pane('/tmp'), opts)
    T.eq(#env.stub.spawned, 2, 'collect = false never spawns')
  end)
end)

T.test('collector: a failed start (no binary) is remembered and not retried every second', function()
  with_status(function(env)
    local attempts = 0
    env.stub.wezterm.background_child_process = function()
      attempts = attempts + 1
      error('No such file or directory')
    end
    local dir = env.stub.tmpdir()
    local opts = status_opts(env, dir, { 'clock' })
    local win = make_window(env.stub)
    env.status.update(win, make_pane('/tmp'), opts)
    env.tick()
    env.status.update(win, make_pane('/tmp'), opts)
    env.tick()
    env.status.update(win, make_pane('/tmp'), opts)
    T.eq(attempts, 1)
  end)
end)

T.test('setup: registers update-status, sets the interval unless the user owns it', function()
  with_status(function(env)
    local resolved = {
      parts = {
        status = { style = 'pill', segments = { 'clock' } },
        palette = { ui = UI },
        font = { effective = { 'Menlo' } },
      },
    }
    local dirs = { state = env.stub.tmpdir() }
    local config = {}
    local summary = env.status.setup(config, { resolved = resolved, dirs = dirs, owned_set = {} })
    T.eq(config.status_update_interval, 1000)
    T.eq(summary.style, 'pill')
    T.eq(#env.stub.handlers['update-status'], 1)

    local owned = {}
    env.status.setup(owned, { resolved = resolved, dirs = dirs, owned_set = { status_update_interval = true } })
    T.eq(owned.status_update_interval, nil, 'a user-owned interval is left alone')

    T.eq(env.status.setup({}, { resolved = { parts = {} }, dirs = dirs }), nil, 'no status part: nothing to do')
  end)
end)
