-- Platform facts and small OS helpers: where files live, which optional config
-- keys this WezTerm knows, how to touch wezterm.GLOBAL safely, and how to
-- write a file atomically.
--
-- Nothing here writes a file or runs a process unless the caller asks it to.

local wezterm = require 'wezterm'

local M = {}

--- Oldest WezTerm release the engine supports (date part of wezterm.version).
M.MIN_WEZTERM = '20240203'

---------------------------------------------------------------------------
-- OS and version
---------------------------------------------------------------------------

--- 'macos', 'linux' or 'windows', from wezterm.target_triple.
function M.os()
  local t = wezterm.target_triple or ''
  if t:find('windows', 1, true) then
    return 'windows'
  elseif t:find('apple', 1, true) then
    return 'macos'
  end
  return 'linux'
end

--- The 8-digit date at the front of wezterm.version, as a number, or nil.
function M.version_date()
  return tonumber(tostring(wezterm.version or ''):match('^(%d%d%d%d%d%d%d%d)'))
end

--- True when the running WezTerm is at least MIN_WEZTERM. Unknown versions
--- (a custom build, say) are given the benefit of the doubt.
function M.meets_minimum()
  local d = M.version_date()
  return d == nil or d >= tonumber(M.MIN_WEZTERM)
end

--- Does wezterm.serde exist (nightly only)?
function M.has_serde()
  return type(wezterm.serde) == 'table' and type(wezterm.serde.json_decode) == 'function'
end

--- Wall clock in seconds. Tests replace M._clock.
function M.now()
  if M._clock then
    return M._clock()
  end
  return os.time()
end

---------------------------------------------------------------------------
-- Optional config keys
---------------------------------------------------------------------------

--- Does this WezTerm accept `key` in a config? wezterm.config_builder() rejects
--- unknown names at assignment, which makes it a probe that needs no
--- version table. Returns true when no builder is available.
function M.config_key_supported(key, sample)
  if type(wezterm.config_builder) ~= 'function' then
    return true
  end
  local ok = pcall(function()
    local c = wezterm.config_builder()
    c[key] = sample
  end)
  return ok
end

--- The config key that blurs the window background here, or nil with a reason.
--- macOS has had its key for years; the Linux keys exist only in nightly
--- builds, so they are probed rather than assumed.
function M.blur_key()
  local os_name = M.os()
  local candidates
  if os_name == 'macos' then
    candidates = { 'macos_window_background_blur' }
  elseif os_name == 'linux' then
    candidates = { 'kde_window_background_blur', 'wayland_window_background_blur' }
  else
    return nil, 'window blur is not available on Windows'
  end
  for _, key in ipairs(candidates) do
    if M.config_key_supported(key, 10) then
      return key
    end
  end
  return nil, 'this WezTerm build has no window background blur key (' .. table.concat(candidates, ', ') .. ')'
end

--- Everything the engine wants to know about the host, in one table.
function M.info()
  local blur_key, blur_reason = M.blur_key()
  return {
    os = M.os(),
    version = wezterm.version,
    meets_minimum = M.meets_minimum(),
    serde = M.has_serde(),
    blur_key = blur_key,
    blur_reason = blur_reason,
  }
end

---------------------------------------------------------------------------
-- Paths
---------------------------------------------------------------------------

function M.join(...)
  return (table.concat({ ... }, '/'):gsub('//+', '/'))
end

function M.dirname(path)
  return (path:gsub('[/\\][^/\\]*$', ''))
end

--- The directories the engine reads and writes.
---   local  user-authored layer (presets, themes, overrides.json, machine.json)
---   fleet  git clone of the private fleet repo (same layout)
---   state  state.json (watched) and screens.json (not watched)
---   data   generated art under <data>/art/<theme>/<W>x<H>/
--- XDG on macOS and Linux (macOS included, on purpose); Known Folders-ish on
--- Windows. `opts.dirs` overrides any of them. Keep crates/wzt-model/paths.rs
--- in step with this function.
function M.dirs(opts)
  local given = (opts and opts.dirs) or {}
  local home = wezterm.home_dir or os.getenv('HOME') or ''
  local function env(name)
    local v = os.getenv(name)
    if v and v ~= '' then
      return v
    end
  end

  local config_root, data_root, state_root
  if M.os() == 'windows' then
    local appdata = env('APPDATA') or M.join(home, 'AppData/Roaming')
    local local_appdata = env('LOCALAPPDATA') or M.join(home, 'AppData/Local')
    config_root = M.join(appdata, 'wezterminator')
    data_root = M.join(local_appdata, 'wezterminator')
    state_root = M.join(local_appdata, 'wezterminator', 'state')
  else
    config_root = M.join(env('XDG_CONFIG_HOME') or M.join(home, '.config'), 'wezterminator')
    data_root = M.join(env('XDG_DATA_HOME') or M.join(home, '.local/share'), 'wezterminator')
    state_root = M.join(env('XDG_STATE_HOME') or M.join(home, '.local/state'), 'wezterminator')
  end

  return {
    ['local'] = given['local'] or config_root,
    fleet = given.fleet or M.join(data_root, 'fleet'),
    state = given.state or state_root,
    data = given.data or data_root,
  }
end

---------------------------------------------------------------------------
-- Files
---------------------------------------------------------------------------

function M.read_file(path)
  local f = io.open(path, 'rb')
  if not f then
    return nil
  end
  local text = f:read('*a')
  f:close()
  return text
end

function M.file_exists(path)
  local f = io.open(path, 'rb')
  if f then
    f:close()
    return true
  end
  return false
end

function M.mkdir_p(path)
  if not (wezterm.run_child_process and path and path ~= '') then
    return false
  end
  local args
  if M.os() == 'windows' then
    args = { 'cmd', '/c', 'mkdir', (path:gsub('/', '\\')) }
  else
    args = { 'mkdir', '-p', path }
  end
  local ok = pcall(wezterm.run_child_process, args)
  return ok
end

--- Write `text` to `path` through a temp file and os.rename, so a reader never
--- sees a half-written document. Returns true, or false and a reason.
function M.write_atomic(path, text)
  local tmp = string.format('%s.tmp%d%d', path, os.time(), math.random(100000, 999999))
  local f = io.open(tmp, 'wb')
  if not f then
    M.mkdir_p(M.dirname(path))
    f = io.open(tmp, 'wb')
  end
  if not f then
    return false, 'cannot open ' .. tmp
  end
  f:write(text)
  f:close()
  local ok, err = os.rename(tmp, path)
  if not ok then
    -- Windows refuses to rename over an existing file.
    os.remove(path)
    ok, err = os.rename(tmp, path)
  end
  if not ok then
    os.remove(tmp)
    return false, tostring(err)
  end
  return true
end

---------------------------------------------------------------------------
-- wezterm.GLOBAL
---------------------------------------------------------------------------

-- wezterm.GLOBAL values come back as copies or proxies, and mutating a nested
-- value in place is not reliable. The rule used everywhere in the engine:
-- read a top-level key into a plain Lua tree, change the tree, write the whole
-- key back. Keys are strings only (window ids are stringified) so tables never
-- flip between array and object.

local function plain(v)
  local t = type(v)
  if t == 'table' then
    local out = {}
    for k, x in pairs(v) do
      out[k] = plain(x)
    end
    return out
  elseif t == 'userdata' then
    local ok, iter, state, init = pcall(pairs, v)
    if not ok then
      return nil
    end
    local out = {}
    for k, x in iter, state, init do
      out[k] = plain(x)
    end
    return out
  elseif t == 'function' or t == 'thread' then
    return nil
  end
  return v
end
M.plain = plain

function M.store_get(key)
  local ok, v = pcall(function()
    return wezterm.GLOBAL[key]
  end)
  if not ok then
    return nil
  end
  return plain(v)
end

function M.store_set(key, value)
  wezterm.GLOBAL[key] = value
end

return M
