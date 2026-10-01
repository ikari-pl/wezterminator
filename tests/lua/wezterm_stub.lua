-- A stand-in for the `wezterm` module, faithful where the engine's correctness
-- depends on WezTerm's real behaviour:
--
--   * wezterm.GLOBAL copies on every read and every write. Mutating a nested
--     table you just read does NOT persist, and metatables (the array marker)
--     are lost. Code that forgets this fails here the way it would in WezTerm.
--   * Every config evaluation runs in a FRESH Lua state: module locals are gone,
--     event handlers are gone, only GLOBAL survives. `stub.eval` reproduces that
--     by clearing package.loaded and the handler table.
--   * wezterm.json_parse is lossy: nulls vanish and empty arrays are plain
--     empty tables.
--   * window:set_config_overrides re-evaluates the config and then emits
--     window-config-reloaded for that window, which is what makes a careless
--     aggregator loop. `stub.attach` enables that emulation, with a depth guard.
--   * config_builder() rejects unknown keys on assignment, which is how
--     platform.lua probes for optional config keys.

local json = require 'json'

local M = {}

local function deep_copy(v, seen)
  if type(v) ~= 'table' then
    if type(v) == 'function' or type(v) == 'userdata' or type(v) == 'thread' then
      error('wezterm.GLOBAL cannot hold a ' .. type(v), 3)
    end
    return v
  end
  local out = {}
  for k, x in pairs(v) do
    out[deep_copy(k)] = deep_copy(x)
  end
  return out -- metatables intentionally dropped
end
M.deep_copy = deep_copy

local function shell_quote(s)
  return "'" .. tostring(s):gsub("'", "'\\''") .. "'"
end
M.shell_quote = shell_quote

--- Create a fresh stub. `opts`:
---   target_triple  default 'aarch64-apple-darwin'
---   version        default '20240203-110809-5046fc22'
---   home_dir       default a temp dir
---   serde          true to expose wezterm.serde.json_decode
---   gui            false to emulate the mux server (wezterm.gui == nil)
---   screens        list of {name,width,height} for wezterm.gui.screens()
---   config_keys    set of valid config keys for config_builder (default: a
---                  macOS-ish list with only macos_window_background_blur)
function M.new(opts)
  opts = opts or {}
  local stub = {
    handlers = {},
    watched = {},
    toasts = {},
    logs = {},
    windows = {},
    io_writes = {},
    renames = {},
    _store = {},
  }

  local wezterm = {}
  stub.wezterm = wezterm

  wezterm.target_triple = opts.target_triple or 'aarch64-apple-darwin'
  wezterm.version = opts.version or '20240203-110809-5046fc22'
  wezterm.home_dir = opts.home_dir or os.getenv('TMPDIR') or '/tmp'
  wezterm.config_dir = wezterm.home_dir .. '/.config/wezterm'

  -- GLOBAL: copy in, copy out.
  wezterm.GLOBAL = setmetatable({}, {
    __index = function(_, k)
      return deep_copy(stub._store[k])
    end,
    __newindex = function(_, k, v)
      stub._store[k] = deep_copy(v)
    end,
  })

  function wezterm.json_parse(text)
    return json.decode(text) -- nulls dropped, no array marker
  end
  if opts.serde then
    wezterm.serde = {
      json_decode = function(text)
        return json.decode(text)
      end,
    }
  end

  function wezterm.on(event, fn)
    stub.handlers[event] = stub.handlers[event] or {}
    table.insert(stub.handlers[event], fn)
  end

  function wezterm.add_to_config_reload_watch_list(path)
    stub.watched[#stub.watched + 1] = path
  end

  stub.reloads = 0
  function wezterm.reload_configuration()
    stub.reloads = stub.reloads + 1
  end

  function wezterm.glob(pattern)
    local p = io.popen('ls -d ' .. pattern:gsub('([^%w%*%?%[%]/%._%-])', '\\%1') .. ' 2>/dev/null')
    local out = {}
    if p then
      for line in p:lines() do
        out[#out + 1] = line
      end
      p:close()
    end
    return out
  end

  function wezterm.run_child_process(args)
    local parts = {}
    for _, a in ipairs(args) do
      parts[#parts + 1] = shell_quote(a)
    end
    local ok = os.execute(table.concat(parts, ' ') .. ' >/dev/null 2>&1')
    return ok == true or ok == 0, '', ''
  end

  stub.plugin_list = opts.plugin_list or {}
  wezterm.plugin = {
    list = function()
      return stub.plugin_list
    end,
  }

  function wezterm.font_with_fallback(list)
    return { font_with_fallback = list }
  end
  function wezterm.font(name)
    return { font = name }
  end
  function wezterm.action_callback(fn)
    return { callback = fn }
  end
  wezterm.action = setmetatable({}, {
    __index = function(_, name)
      return function(arg)
        return { action = name, arg = arg }
      end
    end,
  })

  for _, level in ipairs({ 'info', 'warn', 'error' }) do
    wezterm['log_' .. level] = function(msg)
      stub.logs[#stub.logs + 1] = { level = level, msg = tostring(msg) }
    end
  end

  -- Optional config keys, probed by platform.lua.
  stub.config_keys = opts.config_keys or { macos_window_background_blur = true }
  function wezterm.config_builder()
    return setmetatable({}, {
      __newindex = function(t, k, v)
        if not stub.config_keys[k] then
          error('Config does not have field ' .. tostring(k), 2)
        end
        rawset(t, k, v)
      end,
    })
  end

  -- Builtin colour schemes, for wezterm_scheme expansion.
  stub.builtin_schemes = opts.builtin_schemes or {
    GruvboxDarkHard = {
      foreground = '#ebdbb2',
      background = '#1d2021',
      cursor_bg = '#ebdbb2',
      cursor_fg = '#1d2021',
      selection_bg = '#ebdbb2',
      selection_fg = '#1d2021',
      ansi = { '#1d2021', '#cc241d', '#98971a', '#d79921', '#458588', '#b16286', '#689d6a', '#a89984' },
      brights = { '#928374', '#fb4934', '#b8bb26', '#fabd2f', '#83a598', '#d3869b', '#8ec07c', '#ebdbb2' },
    },
  }
  wezterm.color = {
    get_builtin_schemes = function()
      return stub.builtin_schemes
    end,
  }

  -- wezterm.gui is nil inside the mux server.
  if opts.gui ~= false then
    stub.screens = opts.screens or {}
    wezterm.gui = {
      screens = function()
        local by_name = {}
        for _, s in ipairs(stub.screens) do
          by_name[s.name or 'screen'] = s
        end
        return { by_name = by_name, active = stub.screens[1], main = stub.screens[1] }
      end,
      gui_windows = function()
        local out = {}
        for _, w in ipairs(stub.windows) do
          if not w.closed then
            out[#out + 1] = w
          end
        end
        return out
      end,
    }
  end

  ---------------------------------------------------------------------------
  -- Windows
  ---------------------------------------------------------------------------

  local next_id = 1
  function stub.new_window()
    local w = { id = next_id, writes = 0, _overrides = nil, closed = false }
    next_id = next_id + 1
    function w:window_id()
      return self.id
    end
    function w:get_config_overrides()
      return deep_copy(self._overrides) -- nil when never set
    end
    function w:set_config_overrides(t)
      self.writes = self.writes + 1
      self._overrides = deep_copy(t)
      stub.total_writes = (stub.total_writes or 0) + 1
      if stub._reeval then
        stub._reeval(self)
      end
    end
    function w:toast_notification(title, message, url, ms)
      stub.toasts[#stub.toasts + 1] = { title = title, message = message, window = self.id }
    end
    function w:close()
      self.closed = true
    end
    stub.windows[#stub.windows + 1] = w
    return w
  end

  --- Fire an event at every handler registered in the CURRENT Lua state.
  function stub.emit(event, ...)
    local list = stub.handlers[event]
    if not list then
      return
    end
    local snapshot = { table.unpack and table.unpack(list) or unpack(list) }
    for _, fn in ipairs(snapshot) do
      fn(...)
    end
  end

  --- Emit window-config-reloaded for one window, or for every open window.
  function stub.emit_reloaded(window)
    if window then
      stub.emit('window-config-reloaded', window, nil)
      return
    end
    for _, w in ipairs({ table.unpack and table.unpack(stub.windows) or unpack(stub.windows) }) do
      if not w.closed then
        stub.emit('window-config-reloaded', w, nil)
      end
    end
  end

  function stub.emit_status(window)
    stub.emit('update-status', window, nil)
  end

  ---------------------------------------------------------------------------
  -- Lua-state emulation
  ---------------------------------------------------------------------------

  --- Drop every module a config evaluation would re-load and every handler.
  --- GLOBAL (stub._store) is untouched, as in WezTerm.
  function stub.fresh_state()
    for name in pairs(package.loaded) do
      if name:match('^wzt%.') or name == 'wzt' or name == 'wezterminator' then
        package.loaded[name] = nil
      end
    end
    stub.handlers = {}
    package.loaded.wezterm = wezterm
  end

  --- Evaluate the plugin in a fresh Lua state. Returns config, handle.
  function stub.eval(root, plugin_opts, config)
    stub.fresh_state()
    config = config or {}
    local plugin = dofile(root .. '/plugin/init.lua')
    local handle = plugin.apply_to_config(config, plugin_opts)
    stub.evals = (stub.evals or 0) + 1
    return config, handle
  end

  --- Make set_config_overrides re-evaluate and emit window-config-reloaded,
  --- the way WezTerm does. `evaluator` is called to redo the evaluation.
  function stub.attach(evaluator)
    local depth = 0
    stub._reeval = function(window)
      depth = depth + 1
      if depth > 12 then
        error('config reload loop: set_config_overrides keeps re-triggering itself')
      end
      local ok, err = pcall(function()
        evaluator()
        stub.emit_reloaded(window)
      end)
      depth = depth - 1
      if not ok then
        error(err, 0)
      end
    end
  end

  function stub.detach()
    stub._reeval = nil
  end

  ---------------------------------------------------------------------------
  -- Filesystem helpers and write tracking
  ---------------------------------------------------------------------------

  function stub.tmpdir()
    local p = io.popen('mktemp -d "${TMPDIR:-/tmp}/wzt-test.XXXXXX"')
    local dir = p:read('*l')
    p:close()
    stub._tmpdirs = stub._tmpdirs or {}
    stub._tmpdirs[#stub._tmpdirs + 1] = dir
    return dir
  end

  function stub.mkdir(path)
    os.execute('mkdir -p ' .. shell_quote(path))
  end

  function stub.write(path, text)
    stub.mkdir((path:gsub('/[^/]*$', '')))
    local f = assert(io.open(path, 'w'))
    f:write(text)
    f:close()
  end

  function stub.read(path)
    local f = io.open(path, 'r')
    if not f then
      return nil
    end
    local t = f:read('*a')
    f:close()
    return t
  end

  function stub.exists(path)
    local f = io.open(path, 'r')
    if f then
      f:close()
      return true
    end
    return false
  end

  function stub.cleanup()
    for _, d in ipairs(stub._tmpdirs or {}) do
      if d and d:match('wzt%-test%.') then
        os.execute('rm -rf ' .. shell_quote(d))
      end
    end
    stub._tmpdirs = {}
    stub.untrack_io()
  end

  --- Record every file opened for writing and every rename, so tests can say
  --- "evaluation wrote nothing".
  function stub.track_io()
    if stub._real_open then
      return
    end
    stub._real_open = io.open
    stub._real_rename = os.rename
    io.open = function(path, mode)
      if mode and mode:match('[wa+]') then
        stub.io_writes[#stub.io_writes + 1] = path
      end
      return stub._real_open(path, mode)
    end
    os.rename = function(a, b)
      stub.renames[#stub.renames + 1] = b
      return stub._real_rename(a, b)
    end
  end

  function stub.untrack_io()
    if stub._real_open then
      io.open = stub._real_open
      os.rename = stub._real_rename
      stub._real_open, stub._real_rename = nil, nil
    end
  end

  function stub.writes_matching(pattern)
    local n = 0
    for _, p in ipairs(stub.io_writes) do
      if p:match(pattern) then
        n = n + 1
      end
    end
    return n
  end

  function stub.reset_io()
    stub.io_writes = {}
    stub.renames = {}
  end

  package.loaded.wezterm = wezterm
  return stub
end

return M
