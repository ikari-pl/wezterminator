-- Loading and encoding JSON documents.
--
-- WezTerm 20240203 can only parse JSON from Lua (wezterm.json_parse, or
-- wezterm.serde.json_decode on nightly). This module reads the three layers,
-- strips `_` comment keys right after decoding so no later module needs to know
-- about them, and caches the parsed result in wezterm.GLOBAL.
--
-- THE CACHE. Every set_config_overrides call re-evaluates the whole config in
-- a fresh Lua state, and WezTerm 20240203 has no file-stat API, so there are no
-- modification times to key a cache on. Instead the cache is keyed by a
-- fingerprint: the raw text of everything that can change at run time (the
-- fleet and local layers and state.json). Reading a few small files is cheap;
-- decoding and stripping them is what the cache saves. When the fingerprint
-- changes, a counter -- the GENERATION -- goes up. The override aggregator
-- uses the generation to tell a real reload (rebuild window overrides) from an
-- override-triggered re-evaluation (leave them alone).
--
-- The built-in layer ships inside the plugin and only changes with a plugin
-- update, so it is read once per generation, not fingerprinted.

local wezterm = require 'wezterm'
local resolve = require 'wzt.resolve'
local platform = require 'wzt.platform'

local M = {}

local CACHE_KEY = 'wzt_data'

---------------------------------------------------------------------------
-- Decoding
---------------------------------------------------------------------------

--- Decode JSON text. Returns the value, or nil and a reason.
function M.decode(text)
  local decoder = platform.has_serde() and wezterm.serde.json_decode or wezterm.json_parse
  local ok, v = pcall(decoder, text)
  if not ok then
    return nil, tostring(v)
  end
  return v
end

--- Decode and strip `_` comments (and null-ish values) in one step.
function M.decode_clean(text)
  local v, err = M.decode(text)
  if v == nil then
    return nil, err or 'empty document'
  end
  return resolve.strip_comments(v)
end

---------------------------------------------------------------------------
-- Encoding
---------------------------------------------------------------------------

local ESCAPES = {
  ['"'] = '\\"', ['\\'] = '\\\\', ['\n'] = '\\n', ['\r'] = '\\r', ['\t'] = '\\t',
  ['\b'] = '\\b', ['\f'] = '\\f',
}

local function encode_string(s)
  return '"' .. s:gsub('[%c"\\]', function(c)
    return ESCAPES[c] or string.format('\\u%04x', c:byte())
  end) .. '"'
end

local function encode_number(n)
  if n ~= n or n == math.huge or n == -math.huge then
    return 'null'
  end
  if n == math.floor(n) and math.abs(n) < 2 ^ 53 then
    return string.format('%d', n)
  end
  return string.format('%.14g', n)
end

local FIRST_KEYS = { 'schema_version', 'id', 'name' }

local function ordered_keys(obj)
  local keys, first = {}, {}
  for _, k in ipairs(FIRST_KEYS) do
    if obj[k] ~= nil then
      first[k] = true
      keys[#keys + 1] = k
    end
  end
  local rest = {}
  for k in pairs(obj) do
    if not first[k] then
      rest[#rest + 1] = k
    end
  end
  table.sort(rest, function(a, b)
    return tostring(a) < tostring(b)
  end)
  for _, k in ipairs(rest) do
    keys[#keys + 1] = k
  end
  return keys
end

local function encode_value(v, level)
  local t = type(v)
  if t == 'string' then
    return encode_string(v)
  elseif t == 'number' then
    return encode_number(v)
  elseif t == 'boolean' then
    return tostring(v)
  elseif t ~= 'table' then
    return 'null'
  end

  local pad = string.rep('  ', level + 1)
  local close = string.rep('  ', level)
  if resolve.is_array(v) then
    if #v == 0 then
      return '[]'
    end
    local scalar = true
    for _, x in ipairs(v) do
      if type(x) == 'table' then
        scalar = false
      end
    end
    local parts = {}
    for _, x in ipairs(v) do
      parts[#parts + 1] = encode_value(x, level + 1)
    end
    if scalar then
      return '[' .. table.concat(parts, ', ') .. ']'
    end
    return '[\n' .. pad .. table.concat(parts, ',\n' .. pad) .. '\n' .. close .. ']'
  end

  if next(v) == nil then
    return '{}'
  end
  local parts = {}
  for _, k in ipairs(ordered_keys(v)) do
    parts[#parts + 1] = encode_string(tostring(k)) .. ': ' .. encode_value(v[k], level + 1)
  end
  return '{\n' .. pad .. table.concat(parts, ',\n' .. pad) .. '\n' .. close .. '}'
end

--- Encode a document as pretty JSON with a stable key order. Empty arrays that
--- Lua cannot mark (they came through wezterm.GLOBAL, say) are restored from
--- the schema first, so `history: []` is never written as `history: {}`.
--- `key` names the root's schema key when it is not a whole document.
function M.encode(value, key)
  return encode_value(resolve.normalize(value, key), 0) .. '\n'
end

---------------------------------------------------------------------------
-- Layers
---------------------------------------------------------------------------

local function glob(pattern)
  local ok, list = pcall(wezterm.glob, pattern)
  if not ok or type(list) ~= 'table' then
    return {}
  end
  local out = {}
  for _, p in ipairs(list) do
    out[#out + 1] = p
  end
  table.sort(out)
  return out
end

--- List a layer's files without reading them:
--- { {kind = 'preset'|'theme'|'overrides'|'machine', path = ...}, ... }
--- `builtin` layers never ship overrides or machine settings, so those are not
--- even looked for.
function M.scan_layer(dir, builtin)
  local files = {}
  if not dir or dir == '' then
    return files
  end
  for _, p in ipairs(glob(dir .. '/presets/*.json')) do
    files[#files + 1] = { kind = 'preset', path = p }
  end
  for _, p in ipairs(glob(dir .. '/themes/*/theme.json')) do
    files[#files + 1] = { kind = 'theme', path = p }
  end
  if not builtin then
    for _, kind in ipairs({ 'overrides', 'machine' }) do
      local p = dir .. '/' .. kind .. '.json'
      if platform.file_exists(p) then
        files[#files + 1] = { kind = kind, path = p }
      end
    end
  end
  return files
end

-- Read every file of a scan. Returns the files with `text` filled in, plus the
-- layer's fingerprint piece.
local function read_files(files)
  local pieces = {}
  for _, f in ipairs(files) do
    f.text = platform.read_file(f.path)
    pieces[#pieces + 1] = f.path .. '\0' .. (f.text or '')
  end
  return table.concat(pieces, '\1')
end

-- Decode a scanned layer. Files that fail to decode are skipped and reported;
-- an unreadable file behaves as if it did not exist.
local function parse_layer(files, errors)
  if #files == 0 then
    return nil
  end
  local layer = { presets = resolve.new_array(), themes = resolve.new_array() }
  for _, f in ipairs(files) do
    if f.text then
      local doc, err = M.decode_clean(f.text)
      if doc == nil then
        errors[#errors + 1] = { path = f.path, error = err }
      elseif f.kind == 'preset' then
        layer.presets[#layer.presets + 1] = doc
      elseif f.kind == 'theme' then
        layer.themes[#layer.themes + 1] = doc
      else
        layer[f.kind] = doc
      end
    end
  end
  return layer
end

---------------------------------------------------------------------------
-- Loading with the generation cache
---------------------------------------------------------------------------

--- Load all layers and state.json.
---
--- `where` = { builtin = <plugin dir>, dirs = platform.dirs() }.
---
--- Returns {
---   layers     = { builtin, fleet, ['local'] },   (each a table or nil)
---   state      = the state document or nil,
---   generation = integer, up by one whenever fleet/local/state change,
---   changed    = true when this call started a new generation,
---   errors     = { {path, error}, ... } for files that failed to decode,
--- }
--- Writes nothing to disk.
function M.load(where)
  local dirs = where.dirs
  local fleet_files = M.scan_layer(dirs.fleet, false)
  local local_files = M.scan_layer(dirs['local'], false)
  local state_path = dirs.state .. '/state.json'

  local fingerprint = table.concat({
    read_files(fleet_files),
    read_files(local_files),
    platform.read_file(state_path) or '',
    where.builtin,
  }, '\2')

  local cache = platform.store_get(CACHE_KEY)
  if cache and cache.fingerprint == fingerprint then
    -- Documents lost their array markers on the way through wezterm.GLOBAL;
    -- strip_comments restores them from the schema.
    return {
      layers = {
        builtin = resolve.strip_comments(cache.layers.builtin),
        fleet = resolve.strip_comments(cache.layers.fleet),
        ['local'] = resolve.strip_comments(cache.layers['local']),
      },
      state = resolve.strip_comments(cache.state),
      generation = cache.generation,
      changed = false,
      errors = cache.errors or {},
    }
  end

  local errors = {}
  local builtin_files = M.scan_layer(where.builtin, true)
  read_files(builtin_files)
  local layers = {
    builtin = parse_layer(builtin_files, errors),
    fleet = parse_layer(fleet_files, errors),
    ['local'] = parse_layer(local_files, errors),
  }

  local state
  local state_text = platform.read_file(state_path)
  if state_text then
    local doc, err = M.decode_clean(state_text)
    if doc == nil then
      errors[#errors + 1] = { path = state_path, error = err }
    else
      state = doc
    end
  end

  local generation = ((cache and cache.generation) or 0) + 1
  platform.store_set(CACHE_KEY, {
    fingerprint = fingerprint,
    generation = generation,
    layers = layers,
    state = state,
    errors = errors,
  })

  return { layers = layers, state = state, generation = generation, changed = true, errors = errors }
end

--- The paths that should trigger a reload when they change. screens.json and
--- engine.json are deliberately NOT here, and neither is the state directory:
--- watching it would put those files on the watch list too.
function M.watch_paths(dirs)
  local paths = { dirs.state .. '/state.json' }
  for _, layer in ipairs({ dirs.fleet, dirs['local'] }) do
    for _, f in ipairs(M.scan_layer(layer, false)) do
      paths[#paths + 1] = f.path
    end
    -- Directories, so a file added later is noticed too.
    paths[#paths + 1] = layer .. '/presets'
    paths[#paths + 1] = layer .. '/themes'
    paths[#paths + 1] = layer .. '/overrides.json'
    paths[#paths + 1] = layer .. '/machine.json'
  end
  return paths
end

return M
