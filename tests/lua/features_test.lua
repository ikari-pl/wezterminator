-- The ported metis features around the status bar and keys (U3): the
-- scheme and font pickers with their non-expiring preview, projects,
-- quick-select, the inspect menus, the palette, and the wiring in
-- apply_to_config. Status is in status_test.lua, keys in keys_test.lua and
-- parallax in parallax_test.lua.

local T = ...
local stub_lib = T.stub

local ROOT
do
  local p = io.popen('cd "' .. T.root .. '" && pwd')
  ROOT = p:read('*l')
  p:close()
end

local SCHEMES = {
  GruvboxDarkHard = { foreground = '#ebdbb2', background = '#1d2021' },
  Dracula = { foreground = '#f8f8f2', background = '#282a36' },
}

---------------------------------------------------------------------------
-- Harness
---------------------------------------------------------------------------

--- Window objects with the methods the features call. Every perform_action is
--- recorded; toasts land in stub.toasts.
local function add_window_methods(stub)
  local orig = stub.new_window
  function stub.new_window()
    local w = orig()
    w.performed = {}
    function w:perform_action(action)
      self.performed[#self.performed + 1] = action
    end
    function w:set_left_status() end
    function w:set_right_status() end
    function w:active_workspace()
      return 'default'
    end
    function w:leader_is_active()
      return false
    end
    return w
  end
end

local function setup(stub_opts, plugin_opts)
  stub_opts = stub_opts or {}
  stub_opts.builtin_schemes = stub_opts.builtin_schemes or SCHEMES
  local stub = stub_lib.new(stub_opts)
  -- Modules capture `wezterm` when first required; start from a clean slate.
  for name in pairs(package.loaded) do
    if name:match('^wzt%.') then
      package.loaded[name] = nil
    end
  end
  add_window_methods(stub)
  local tmp = stub.tmpdir()
  local dirs = {
    ['local'] = tmp .. '/config',
    fleet = tmp .. '/data/fleet',
    state = tmp .. '/state',
    data = tmp .. '/data',
  }
  local opts = { dir = ROOT, dirs = dirs, mode = 'add-on', collect = false }
  for k, v in pairs(plugin_opts or {}) do
    opts[k] = v
  end
  local env = { stub = stub, tmp = tmp, dirs = dirs, opts = opts }
  function env.eval(config)
    local cfg, summary = stub.eval(ROOT, opts, config or {})
    env.config, env.summary = cfg, summary
    return cfg, summary
  end
  function env.ctx()
    return { resolved = env.summary.result.resolved, host = env.summary.host, dirs = dirs }
  end
  return env
end

local function with_env(stub_opts, plugin_opts, fn)
  return function()
    local env = setup(stub_opts, plugin_opts)
    local ok, err = pcall(fn, env)
    env.stub.cleanup()
    package.loaded.wezterm = nil
    if not ok then
      error(err, 0)
    end
  end
end

--- Run the callback of a recorded InputSelector with a pick (or nil: cancel).
local function choose(action, window, id)
  action.arg.action.callback(window, nil, id, nil)
end

local function labels(action)
  local out = {}
  for _, c in ipairs(action.arg.choices) do
    out[#out + 1] = c.label
  end
  return table.concat(out, '\n')
end

local function overrides_doc(env)
  local text = env.stub.read(env.dirs['local'] .. '/overrides.json')
  return text and T.json.decode(text)
end

---------------------------------------------------------------------------
-- Pickers
---------------------------------------------------------------------------

T.test('scheme picker: a pick previews with owner=picker and NO expiry; the confirm menu follows', with_env(nil, nil, function(env)
  env.eval()
  local pickers = require 'wzt.pickers'
  local ov = require 'wzt.overrides'
  local w = env.stub.new_window()

  T.eq(pickers.open(w, nil, 'scheme', env.ctx()), true)
  T.eq(#w.performed, 1)
  T.eq(w.performed[1].action, 'InputSelector')
  T.ok(labels(w.performed[1]):find('Dracula', 1, true), 'the built-in schemes are listed')
  T.eq(w.performed[1].arg.fuzzy, true)

  choose(w.performed[1], w, 'Dracula')
  local ch = ov.get_channel(w, 'preview')
  T.ok(ch, 'preview channel set')
  T.eq(ch.owner, 'picker')
  T.eq(ch.expires_at, nil, 'a Lua picker preview never expires')
  T.eq(w:get_config_overrides().color_scheme, 'Dracula', 'the window shows the previewed scheme')
  T.eq(#w.performed, 2, 'the confirm menu opened')
  T.eq(w.performed[2].arg.choices[1].id, 'keep')

  -- A long time passes (the confirm menu is open): the preview is still there.
  local platform = require 'wzt.platform'
  local t0 = os.time()
  platform._clock = function()
    return t0 + 3600
  end
  ov.tick(w)
  T.ok(ov.get_channel(w, 'preview'), 'status ticks do not expire a picker preview')
  T.eq(w:get_config_overrides().color_scheme, 'Dracula')
  platform._clock = nil
end))

T.test('scheme picker: the preview carries the terminal colours too, or the old ones would sit on top', with_env(nil, nil, function(env)
  env.eval()
  local pickers = require 'wzt.pickers'
  local payload = pickers.scheme_payload(env.ctx().resolved, env.ctx().host, 'Dracula')
  T.eq(payload.config.color_scheme, 'Dracula')
  T.ok(payload.config.colors and payload.config.colors.tab_bar, 'the tab bar colours are kept')
  T.eq(payload.config.colors.foreground, nil, 'no terminal colours that would override the scheme')
end))

T.test('scheme picker: Keep writes overrides.json; the reload retires the preview and shows the scheme', with_env(nil, nil, function(env)
  env.eval()
  local pickers = require 'wzt.pickers'
  local ov = require 'wzt.overrides'
  local w = env.stub.new_window()
  pickers.open(w, nil, 'scheme', env.ctx())
  choose(w.performed[1], w, 'Dracula')
  choose(w.performed[2], w, 'keep')

  local doc = overrides_doc(env)
  T.ok(doc, 'overrides.json written')
  T.eq(doc.schema_version, 1)
  T.eq(doc.parts.scheme.wezterm_scheme, 'Dracula')
  T.ok(env.stub.toasts[#env.stub.toasts].message:find('Dracula', 1, true))

  -- The watched file changed: the next evaluation is a new generation.
  local config = env.eval()
  T.eq(config.color_scheme, 'Dracula', 'the base config now has the scheme')
  env.stub.emit_reloaded(w)
  T.eq(ov.get_channel(w, 'preview'), nil, 'a commit retires the preview')
  T.eq(w:get_config_overrides().color_scheme, nil, 'and the window no longer needs the override')
end))

T.test('scheme picker: Revert, Esc on the confirm menu and Esc on the list all clear the preview', with_env(nil, nil, function(env)
  env.eval()
  local pickers = require 'wzt.pickers'
  local ov = require 'wzt.overrides'
  local w = env.stub.new_window()

  pickers.open(w, nil, 'scheme', env.ctx())
  choose(w.performed[1], w, 'Dracula')
  choose(w.performed[2], w, 'revert')
  T.eq(ov.get_channel(w, 'preview'), nil, 'revert')
  T.eq(w:get_config_overrides().color_scheme, nil)

  pickers.open(w, nil, 'scheme', env.ctx())
  choose(w.performed[3], w, 'Dracula')
  choose(w.performed[4], w, nil)
  T.eq(ov.get_channel(w, 'preview'), nil, 'Esc on the confirm menu')

  pickers.open(w, nil, 'scheme', env.ctx())
  choose(w.performed[5], w, 'Dracula')
  choose(w.performed[6], w, 'another')
  T.ok(ov.get_channel(w, 'preview'), 'Choose another keeps the current preview while the list is open')
  choose(w.performed[7], w, nil)
  T.eq(ov.get_channel(w, 'preview'), nil, 'Esc on the re-opened list')

  T.eq(overrides_doc(env), nil, 'nothing was saved')
end))

T.test('scheme picker: a failed save clears the preview and says why', with_env(nil, nil, function(env)
  env.eval()
  env.stub.write(env.dirs['local'] .. '/overrides.json', '{ this is not json')
  local pickers = require 'wzt.pickers'
  local ov = require 'wzt.overrides'
  local w = env.stub.new_window()
  pickers.open(w, nil, 'scheme', env.ctx())
  choose(w.performed[1], w, 'Dracula')
  choose(w.performed[2], w, 'keep')
  T.eq(env.stub.read(env.dirs['local'] .. '/overrides.json'), '{ this is not json', 'a corrupt file is not overwritten')
  T.eq(ov.get_channel(w, 'preview'), nil)
  T.ok(env.stub.toasts[#env.stub.toasts].message:find('Cannot save', 1, true))
end))

T.test('scheme picker: committing keeps comments and unrelated parts of overrides.json', with_env(nil, nil, function(env)
  env.eval()
  env.stub.write(env.dirs['local'] .. '/overrides.json', [[{
  "schema_version": 1,
  "_": "my tweaks",
  "parts": { "chrome": { "opacity": 0.8 }, "_note": "keep" }
}]])
  local pickers = require 'wzt.pickers'
  T.eq(pickers.commit_scheme(env.dirs, 'Dracula'), true)
  local text = env.stub.read(env.dirs['local'] .. '/overrides.json')
  local doc = T.json.decode(text)
  T.eq(doc.parts.chrome.opacity, 0.8)
  T.eq(doc.parts.scheme.wezterm_scheme, 'Dracula')
  T.eq(doc['_'], 'my tweaks')
  T.eq(doc.parts['_note'], 'keep')
end))

T.test('font picker: previews the family with the preset fallback list and size rules', with_env(nil, nil, function(env)
  env.eval()
  local pickers = require 'wzt.pickers'
  local ov = require 'wzt.overrides'
  local ctx = env.ctx()
  local w = env.stub.new_window()
  T.eq(pickers.open(w, nil, 'font', ctx), true)
  local font = ctx.resolved.parts.font
  local first = font.preferred[1]
  T.ok(labels(w.performed[1]):find(first, 1, true), 'the preset fonts are listed')

  local other = font.fallback[1]
  choose(w.performed[1], w, other)
  local ch = ov.get_channel(w, 'preview')
  T.eq(ch.owner, 'picker')
  T.eq(ch.expires_at, nil)
  local o = w:get_config_overrides()
  T.eq(o.font.font_with_fallback[1], other, 'the chosen family comes first')
  T.ok(o.font_size, 'the size rule applies')

  choose(w.performed[2], w, 'keep')
  local doc = overrides_doc(env)
  T.eq(doc.parts.font.preferred[1], other)
  T.eq(#doc.parts.font.preferred, 1)
end))

T.test('add-on: when the user config owns the colour scheme the picker is off and says so', with_env(nil, nil, function(env)
  env.eval({ color_scheme = 'Mine' })
  T.eq(env.summary.result.resolved.parts.scheme, nil, 'the scheme part was overruled')
  local pickers = require 'wzt.pickers'
  local w = env.stub.new_window()
  T.eq(pickers.open(w, nil, 'scheme', env.ctx()), false)
  T.eq(#w.performed, 0, 'no menu')
  T.ok(env.stub.toasts[#env.stub.toasts].message:find('picker is off', 1, true))
end))

---------------------------------------------------------------------------
-- Projects
---------------------------------------------------------------------------

local function fake_glob(tree)
  return function(pattern)
    local out = {}
    for _, path in ipairs(tree) do
      -- '<root>/*/.git' matches one level, '<root>/*/*/.git' two.
      local root, depth = pattern:match('^(.*)/%*/%*/%.git$'), 2
      if not root then
        root, depth = pattern:match('^(.*)/%*/%.git$'), 1
      end
      if root and path:sub(1, #root + 1) == root .. '/' then
        local rest = path:sub(#root + 2):gsub('/%.git$', '')
        local _, slashes = rest:gsub('/', '')
        if slashes == depth - 1 then
          out[#out + 1] = path
        end
      end
    end
    return out
  end
end

T.test('projects: no personal defaults -- with no project_roots there is nothing to scan', with_env(nil, nil, function(env)
  local projects = require 'wzt.projects'
  T.eq(#projects.roots({}, '/home/x'), 0)
  T.eq(#projects.roots(nil, '/home/x'), 0)
  local w = env.stub.new_window()
  T.eq(projects.pick(w, nil, {}, { home = '/home/x' }), false)
  T.eq(#w.performed, 0)
  T.ok(env.stub.toasts[#env.stub.toasts].message:find('project_roots', 1, true), 'says how to configure it')
end))

T.test('projects: roots expand ~/ and discovery finds one and two levels, sorted', with_env(nil, nil, function(env)
  local projects = require 'wzt.projects'
  local roots = projects.roots({ project_roots = { '~/code', '/srv/git/' } }, '/home/x')
  T.eq(roots[1], '/home/x/code')
  T.eq(roots[2], '/srv/git')

  local found = projects.discover({ '/home/x/code' }, fake_glob({
    '/home/x/code/zeta/.git',
    '/home/x/code/ai/agent/.git',
    '/home/x/code/alpha/.git',
  }))
  T.eq(#found, 3)
  T.eq(found[1].workspace, 'ai/agent')
  T.eq(found[2].workspace, 'alpha')
  T.eq(found[3].workspace, 'zeta')
  T.eq(found[1].path, '/home/x/code/ai/agent')
end))

T.test('projects: choosing a repository switches to a workspace spawned in it', with_env(nil, nil, function(env)
  local projects = require 'wzt.projects'
  local w = env.stub.new_window()
  local opts = {
    home = '/home/x',
    glob = fake_glob({ '/home/x/code/alpha/.git', '/home/x/code/ai/agent/.git' }),
  }
  T.eq(projects.pick(w, nil, { project_roots = { '~/code' } }, opts), true)
  local selector = w.performed[1]
  T.eq(#selector.arg.choices, 2)
  choose(selector, w, '/home/x/code/ai/agent')
  local switch = w.performed[2]
  T.eq(switch.action, 'SwitchToWorkspace')
  T.eq(switch.arg.name, 'ai/agent')
  T.eq(switch.arg.spawn.cwd, '/home/x/code/ai/agent')
  choose(selector, w, nil)
  T.eq(#w.performed, 2, 'cancelling does nothing')
end))

---------------------------------------------------------------------------
-- Quick-select
---------------------------------------------------------------------------

T.test('quick-select: an issue key resolves through the configured URL pattern', with_env(nil, nil, function()
  local qs = require 'wzt.quickselect'
  local r = qs.resolve('ABC-42', { issue_url_pattern = 'https://tracker.example/browse/{key}' })
  T.eq(r.kind, 'issue')
  T.eq(r.open, 'https://tracker.example/browse/ABC-42')
  -- A pattern with a % in it must not break the substitution.
  local r2 = qs.resolve('ABC-42', { issue_url_pattern = 'https://t.example/%20{key}?a=1%' })
  T.eq(r2.open, 'https://t.example/%20ABC-42?a=1%')
end))

T.test('quick-select: with no URL pattern the issue key is only copied -- there is no built-in tracker', with_env(nil, nil, function()
  local qs = require 'wzt.quickselect'
  local r = qs.resolve('ABC-42', {})
  T.eq(r.copy, 'ABC-42')
  T.eq(r.open, nil)
  T.ok(r.note:find('issue_url_pattern', 1, true))
end))

T.test('quick-select: URLs open, SHAs go to the GitHub origin or are copied', with_env(nil, nil, function()
  local qs = require 'wzt.quickselect'
  T.eq(qs.resolve('https://example.com/a?b=1', {}).open, 'https://example.com/a?b=1')
  T.eq(qs.classify('deadbeef1'), 'sha')
  T.eq(qs.classify('ABC-1234567'), 'issue', 'a dash makes it an issue key even when long')
  T.eq(qs.classify('abc123'), 'issue', 'six hex characters is too short for a SHA')

  local calls = 0
  local with_repo = qs.resolve('deadbeef1', {}, function()
    calls = calls + 1
    return 'https://github.com/o/r'
  end)
  T.eq(with_repo.open, 'https://github.com/o/r/commit/deadbeef1')
  local without = qs.resolve('deadbeef1', {}, function()
    return nil
  end)
  T.eq(without.copy, 'deadbeef1')

  local lazy = 0
  qs.resolve('ABC-1', {}, function()
    lazy = lazy + 1
  end)
  T.eq(lazy, 0, 'git is only asked for when a SHA was selected')
  T.eq(qs.resolve('   ', {}), nil)
end))

T.test('quick-select: GitHub remotes in all three forms', with_env(nil, nil, function()
  local qs = require 'wzt.quickselect'
  T.eq(qs.github_url_from_remote('git@github.com:o/r.git\n'), 'https://github.com/o/r')
  T.eq(qs.github_url_from_remote('ssh://git@github.com/o/r'), 'https://github.com/o/r')
  T.eq(qs.github_url_from_remote('https://github.com/o/r.git'), 'https://github.com/o/r')
  T.eq(qs.github_url_from_remote('git@gitlab.com:o/r.git'), nil)
end))

T.test('quick-select: the key pattern comes from machine settings, with a generic default', with_env(nil, nil, function()
  local qs = require 'wzt.quickselect'
  T.eq(qs.patterns({})[3], qs.DEFAULT_ISSUE_KEY)
  T.eq(qs.patterns({ issue_key_pattern = [[\bDEV-\d+\b]] })[3], [[\bDEV-\d+\b]])
  T.eq(#qs.patterns({}), 3)
end))

T.test('quick-select: the action opens a URL, and copies when there is nothing to open', with_env(nil, nil, function(env)
  local opened = {}
  env.stub.wezterm.open_with = function(url)
    opened[#opened + 1] = url
  end
  local qs = require 'wzt.quickselect'
  local actions = qs.actions({ machine = { issue_url_pattern = 'https://t.example/{key}' } })
  local callback = actions.quick_select.arg.action.callback

  local w = env.stub.new_window()
  function w:get_selection_text_for_pane()
    return ' ABC-7 '
  end
  callback(w, nil)
  T.eq(opened[1], 'https://t.example/ABC-7')

  local actions2 = qs.actions({ machine = {} })
  actions2.quick_select.arg.action.callback(w, nil)
  T.eq(#opened, 1, 'no pattern: nothing opened')
  T.eq(w.performed[#w.performed].action, 'CopyTo')
  T.eq(w.performed[#w.performed].arg, 'Clipboard')
end))

T.test('editor: the command and args come from machine settings only', with_env(nil, nil, function()
  local qs = require 'wzt.quickselect'
  local args = qs.editor_args({ editor = { command = 'nvim', args = { '-R' } } }, '/c/overrides.json')
  T.eq(table.concat(args, ' '), 'nvim -R /c/overrides.json')
  local none, why = qs.editor_args({}, '/c/overrides.json')
  T.eq(none, nil)
  T.ok(why:find('editor', 1, true))
end))

---------------------------------------------------------------------------
-- Inspect menus
---------------------------------------------------------------------------

T.test('inspect: ps flags per platform, none on Windows', with_env(nil, nil, function()
  local inspect = require 'wzt.inspect'
  T.eq(table.concat(inspect.ps_command('macos', 'cpu'), ' '), 'ps -Aceo pid,%cpu,rss,comm -r')
  T.eq(table.concat(inspect.ps_command('macos', 'mem'), ' '), 'ps -Aceo pid,%cpu,rss,comm -m')
  T.eq(inspect.ps_command('linux', 'cpu')[4], '--sort=-pcpu')
  T.eq(inspect.ps_command('linux', 'mem')[4], '--sort=-rss')
  T.eq(inspect.ps_command('windows', 'cpu'), nil)
end))

T.test('inspect: ps output parses to rows, skipping the header and capping the count', with_env(nil, nil, function()
  local inspect = require 'wzt.inspect'
  local out = table.concat({
    '  PID  %CPU    RSS COMM',
    '  412  93.5 2097152 node',
    '    1   0.1   10240 launchd',
    'garbage line',
    '  900   1.0 524288 Google Chrome Helper',
  }, '\n')
  local rows = inspect.parse_ps(out, 10)
  T.eq(#rows, 3)
  T.eq(rows[1].pid, '412')
  T.eq(rows[1].gb, 2.0)
  T.eq(rows[3].comm, 'Google Chrome Helper')
  T.eq(#inspect.parse_ps(out, 1), 1)
end))

T.test('inspect: on a platform without ps the menu explains instead of failing', with_env(nil, nil, function(env)
  local inspect = require 'wzt.inspect'
  local w = env.stub.new_window()
  T.eq(inspect.show_processes(w, nil, 'cpu', 'Top', 'windows'), false)
  T.eq(#w.performed, 0)
  T.ok(env.stub.toasts[#env.stub.toasts].message:find('ps', 1, true))
end))

T.test('inspect: the process menu lists rows and kill is a deliberate second choice', with_env(nil, nil, function(env)
  local spawned = {}
  env.stub.wezterm.background_child_process = function(args)
    spawned[#spawned + 1] = args
  end
  env.stub.wezterm.run_child_process = function()
    return true, '  PID  %CPU    RSS COMM\n  412  93.5 2097152 node\n', ''
  end
  local inspect = require 'wzt.inspect'
  local w = env.stub.new_window()
  T.eq(inspect.show_processes(w, nil, 'cpu', 'Top by CPU', 'macos'), true)
  local list = w.performed[1]
  T.ok(labels(list):find('node', 1, true))
  choose(list, w, '412')
  local second = w.performed[2]
  T.eq(#spawned, 0, 'picking a process kills nothing')
  T.eq(second.arg.choices[3].id, 'term')
  choose(second, w, 'term')
  T.eq(table.concat(spawned[1], ' '), 'kill -TERM 412')
end))

---------------------------------------------------------------------------
-- Wiring in apply_to_config
---------------------------------------------------------------------------

T.test('apply_to_config installs the features: keys, status handler, palette with every preset', with_env(nil, nil, function(env)
  local config, summary = env.eval()
  local f = summary.features
  T.ok(f.keys and #f.keys.added > 20, 'engine keys added')
  T.eq(#f.keys.conflicts, 0)
  T.ok(#config.keys > 20)
  T.eq(config.leader.key, 'Space')
  T.eq(config.status_update_interval, 1000)
  T.ok(f.status and f.status.style, 'status installed from the preset')

  -- The palette lists each shipped preset by name.
  local entries = env.stub.handlers['augment-command-palette'][1]()
  local briefs = {}
  for _, e in ipairs(entries) do
    briefs[e.brief] = true
  end
  T.ok(briefs['Preset: Soft Nebula'] and briefs['Preset: Ember'] and briefs['Preset: CPC Cool'], 'a preset entry each')
  T.ok(briefs['Appearance: Choose color scheme'])
  T.ok(briefs['Navigate: Open URL, issue, or git commit'])
  T.eq(f.palette_entries, #entries)

  -- The preset entries commit through the engine.
  local committed
  for _, e in ipairs(entries) do
    if e.brief == 'Preset: Ember' then
      committed = e.action
    end
  end
  T.ok(committed and committed.callback, 'a callback action')
end))

T.test('apply_to_config: engine keys do not touch the user keys, and conflicts are reported by id', with_env(nil, nil, function(env)
  local mine = { key = 'P', mods = 'CMD|SHIFT', action = 'mine' }
  local config, summary = env.eval({ keys = { mine }, leader = { key = 'a', mods = 'CTRL' } })
  T.eq(config.keys[1], mine, 'the user entry is first and untouched')
  T.eq(config.leader.key, 'a')
  local ids = {}
  for _, c in ipairs(summary.features.keys.conflicts) do
    ids[#ids + 1] = c.id
  end
  T.eq(table.concat(ids, ','), 'palette')
  for _, k in ipairs(config.keys) do
    if k ~= mine then
      T.ok(not (k.key == 'P' and k.mods == 'CMD|SHIFT'), 'the engine did not add the clashing chord')
    end
  end
  local warned = false
  for _, l in ipairs(env.stub.logs) do
    if l.msg:find('key conflict (palette)', 1, true) then
      warned = true
    end
  end
  T.ok(warned, 'the conflict is logged')
end))

T.test('apply_to_config: features = false installs nothing and a part can be switched off', with_env(nil, { features = false }, function(env)
  local config, summary = env.eval()
  T.eq(config.keys, nil)
  T.eq(config.status_update_interval, nil)
  T.eq(next(summary.features), nil)
  T.eq(env.stub.handlers['augment-command-palette'], nil)

  env.opts.features = { keys = false, palette = false }
  local config2, summary2 = env.eval()
  T.eq(config2.keys, nil, 'keys off')
  T.eq(summary2.features.keys, nil)
  T.ok(summary2.features.status, 'status still on')
  T.eq(env.stub.handlers['augment-command-palette'], nil, 'palette off')
end))

T.test('apply_to_config: the user status_update_interval is left alone', with_env(nil, nil, function(env)
  local config = env.eval({ status_update_interval = 250 })
  T.eq(config.status_update_interval, 250)
end))

T.test('apply_to_config: evaluating the features writes nothing to disk', with_env(nil, nil, function(env)
  env.stub.track_io()
  env.eval()
  T.eq(env.stub.writes_matching('.'), 0, 'no file opened for writing during evaluation')
end))

T.test('apply_to_config: the project picker and editor read machine settings from the layers', with_env(nil, nil, function(env)
  env.stub.write(env.dirs['local'] .. '/machine.json',
    '{"schema_version": 1, "project_roots": ["~/code"], "issue_url_pattern": "https://t.example/{key}"}')
  local _, summary = env.eval()
  T.deep_eq(summary.result.machine.project_roots, { '~/code' })
  T.eq(summary.result.machine.issue_url_pattern, 'https://t.example/{key}')
end))

T.test('no personal defaults ship in the plugin: no tracker org, home paths or tool locations', function()
  local files = {}
  for _, p in ipairs(T.glob(ROOT .. '/plugin/*.lua')) do
    files[#files + 1] = p
  end
  for _, p in ipairs(T.glob(ROOT .. '/plugin/wzt/*.lua')) do
    files[#files + 1] = p
  end
  for _, p in ipairs(T.glob(ROOT .. '/plugin/wzt/segments/*.lua')) do
    files[#files + 1] = p
  end
  T.ok(#files > 20, 'found the plugin sources')
  local needles = { 'opendoor', '/Users/', '/opt/homebrew', '/usr/local/bin', 'linear.app', '~/src', '/src"' }
  for _, path in ipairs(files) do
    local text = T.read_file(path)
    for _, needle in ipairs(needles) do
      T.ok(not text:find(needle, 1, true), path .. ' contains the personal default ' .. needle)
    end
  end
end)
