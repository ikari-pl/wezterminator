-- The engine's on-disk state, kept in the state directory:
--
--   state.json    active preset, undo history, install mode. WATCHED by
--                 WezTerm. Written by the TUI, the CLI, and the commit / undo /
--                 cycle helpers below -- which only ever run from event
--                 handlers and actions, NEVER during config evaluation.
--   screens.json  device-pixel sizes of the connected screens. NOT watched.
--
-- screens.json is a separate file precisely so that writing it can never
-- trigger a reload: it is not on the watch list, and the watch list names
-- state.json itself, never its directory. It is written from a GUI event and
-- only when its content actually changed. The engine record (plugin directory
-- and version, for the Rust side) lives in state.json's `engine` field per the
-- data model; see record_engine for how that stays loop-free.
--
-- Every write is atomic (temp file, then rename).

local wezterm = require 'wezterm'
local resolve = require 'wzt.resolve'
local data = require 'wzt.data'
local platform = require 'wzt.platform'

local M = {}

M.SCHEMA_VERSION = 1
M.HISTORY_CAP = 20

function M.path(dirs)
  return dirs.state .. '/state.json'
end

local function timestamp()
  return os.date('!%Y-%m-%dT%H:%M:%SZ', platform.now())
end

---------------------------------------------------------------------------
-- state.json
---------------------------------------------------------------------------

--- Read state.json keeping its `_` comments, so a rewrite does not erase
--- hand-written notes. Returns doc, or nil and a reason. A missing file is
--- (nil, 'missing'); an unreadable one is (nil, 'corrupt: ...').
function M.read_raw(dirs)
  local text = platform.read_file(M.path(dirs))
  if not text then
    return nil, 'missing'
  end
  local doc, err = data.decode(text)
  if type(doc) ~= 'table' then
    return nil, 'corrupt: ' .. tostring(err or 'not an object')
  end
  return resolve.normalize(doc)
end

function M.write(dirs, doc)
  return platform.write_atomic(M.path(dirs), data.encode(doc))
end

-- Load the document to modify, or a fresh one. Refuses (returns nil and a
-- reason) to overwrite a corrupt file or one from a newer schema.
local function open_for_update(dirs)
  local doc, why = M.read_raw(dirs)
  if doc == nil then
    if why == 'missing' then
      return {
        schema_version = M.SCHEMA_VERSION,
        history = resolve.new_array(),
      }, false
    end
    return nil, why
  end
  local v = doc.schema_version
  if v ~= nil and v ~= M.SCHEMA_VERSION then
    return nil, 'state.json has schema_version ' .. tostring(v) .. ', this engine writes ' .. M.SCHEMA_VERSION
  end
  doc.schema_version = M.SCHEMA_VERSION
  if type(doc.history) ~= 'table' then
    doc.history = resolve.new_array()
  end
  return doc, true
end

local function save(dirs, doc, existed)
  local ok, err = M.write(dirs, doc)
  if not ok then
    return false, err
  end
  -- A path that did not exist when the watch list was built cannot have been
  -- watched, so the first-ever write would not reload by itself.
  if not existed and wezterm.reload_configuration then
    pcall(wezterm.reload_configuration)
  end
  return true
end

--- Make `preset_id` the active preset. The outgoing preset goes on the undo
--- history (oldest first, capped at 20). Returns true, or false and a reason;
--- committing what is already active is a no-op that writes nothing.
function M.commit(dirs, preset_id)
  local doc, existed = open_for_update(dirs)
  if not doc then
    return false, existed
  end
  if doc.active_preset == preset_id then
    return true, 'unchanged'
  end
  if type(doc.active_preset) == 'string' then
    doc.history[#doc.history + 1] = { preset = doc.active_preset, at = timestamp() }
    while #doc.history > M.HISTORY_CAP do
      table.remove(doc.history, 1)
    end
  end
  doc.active_preset = preset_id
  return save(dirs, doc, existed)
end

--- Undo the last commit: pop the newest history entry and make it active.
--- Returns the preset id now active, or nil and a reason.
function M.undo(dirs)
  local doc, existed = open_for_update(dirs)
  if not doc then
    return nil, existed
  end
  local n = #doc.history
  if n == 0 then
    return nil, 'nothing to undo'
  end
  local entry = table.remove(doc.history, n)
  doc.active_preset = entry.preset
  local ok, err = save(dirs, doc, existed)
  if not ok then
    return nil, err
  end
  return entry.preset
end

--- Commit the preset `step` places after (or before) the active one in `ids`,
--- wrapping around. `ids` is the ordered list the caller wants to cycle through.
function M.cycle(dirs, step, ids, active)
  if #ids == 0 then
    return nil, 'no presets'
  end
  local current = active
  local doc = M.read_raw(dirs)
  if doc and type(doc.active_preset) == 'string' then
    current = doc.active_preset
  end
  local idx = 0
  for i, id in ipairs(ids) do
    if id == current then
      idx = i
      break
    end
  end
  local target = ids[((idx - 1 + step) % #ids) + 1]
  local ok, err = M.commit(dirs, target)
  if not ok then
    return nil, err
  end
  return target
end

---------------------------------------------------------------------------
-- screens.json (unwatched) and the engine record
---------------------------------------------------------------------------

-- Write `doc` to `path` unless the file already holds the same content
-- (comments ignored). Returns true only when something was written.
local function write_if_changed(path, doc, key)
  local text = platform.read_file(path)
  if text then
    local existing = data.decode_clean(text)
    if existing ~= nil and resolve.deep_equal(existing, resolve.strip_comments_as(doc, key)) then
      return false
    end
  end
  return platform.write_atomic(path, data.encode(doc, key)) and true or false
end

--- Normalise a screen list: integer pixel sizes, stable order.
function M.normalize_screens(list)
  local out = {}
  for _, s in ipairs(list) do
    local w, h = tonumber(s.width), tonumber(s.height)
    if w and h and w >= 1 and h >= 1 then
      local entry = { width = math.floor(w), height = math.floor(h) }
      if type(s.name) == 'string' and s.name ~= '' then
        entry.name = s.name
      end
      out[#out + 1] = entry
    end
  end
  table.sort(out, function(a, b)
    if (a.name or '') ~= (b.name or '') then
      return (a.name or '') < (b.name or '')
    end
    if a.width ~= b.width then
      return a.width < b.width
    end
    return a.height < b.height
  end)
  return resolve.new_array(out)
end

function M.screens_path(dirs)
  return dirs.state .. '/screens.json'
end

function M.read_screens(dirs)
  local text = platform.read_file(M.screens_path(dirs))
  if not text then
    return nil
  end
  local doc = data.decode_clean(text)
  if type(doc) ~= 'table' or doc.schema_version ~= M.SCHEMA_VERSION or type(doc.screens) ~= 'table' then
    return nil
  end
  return doc.screens
end

--- Record the connected screens. Returns true only when the file was written.
function M.write_screens(dirs, list)
  local screens = M.normalize_screens(list)
  if #screens == 0 then
    return false
  end
  return write_if_changed(
    M.screens_path(dirs),
    { schema_version = M.SCHEMA_VERSION, screens = screens },
    nil
  )
end

--- Record where the running engine lives, so the Rust side reads the same
--- built-ins the running Lua uses. The contract (docs/data-model.md) keeps this
--- in state.json's `engine` field, which `crates/wzt-model` reads as
--- `State::plugin_dir()`.
---
--- state.json is WATCHED, so this must converge: it writes only when the stored
--- values differ from `info`, and the reload that write causes then finds them
--- equal and writes nothing. It runs from a GUI event, never during config
--- evaluation. Only the `engine` field is touched; the active preset, history
--- and any hand-written `_` comments are kept. If state.json does not exist yet
--- it is created with `default_preset` active, because the schema requires an
--- active preset.
---
--- info = { plugin_dir, version, schema_version, default_preset }
--- Returns true when something was written, false when already current, or
--- false and a reason when the file could not be updated.
function M.record_engine(dirs, info)
  local doc, existed = open_for_update(dirs)
  if not doc then
    return false, existed -- `existed` carries the reason when doc is nil
  end
  local want = {
    plugin_dir = info.plugin_dir,
    version = info.version,
    schema_version = info.schema_version or M.SCHEMA_VERSION,
  }
  local cur = doc.engine
  if existed and type(doc.active_preset) == 'string' and type(cur) == 'table'
    and cur.plugin_dir == want.plugin_dir
    and cur.version == want.version
    and cur.schema_version == want.schema_version then
    return false
  end
  local engine = type(cur) == 'table' and cur or {}
  for k, v in pairs(want) do
    engine[k] = v
  end
  doc.engine = engine
  if type(doc.active_preset) ~= 'string' then
    doc.active_preset = info.default_preset
  end
  if type(doc.active_preset) ~= 'string' then
    return false, 'no active preset to record alongside the engine'
  end
  local ok, err = save(dirs, doc, existed)
  return ok and true or false, err
end

--- The screens as WezTerm reports them right now, or nil when there is no GUI
--- (inside the mux server wezterm.gui is nil) or the call is unavailable.
function M.current_screens()
  if not wezterm.gui or type(wezterm.gui.screens) ~= 'function' then
    return nil
  end
  local ok, info = pcall(wezterm.gui.screens)
  if not ok or type(info) ~= 'table' then
    return nil
  end
  local list = {}
  local by_name = info.by_name
  if type(by_name) == 'table' then
    for name, s in pairs(by_name) do
      list[#list + 1] = { name = s.name or name, width = s.width, height = s.height }
    end
  end
  if #list == 0 and type(info.active) == 'table' then
    list[1] = { name = info.active.name, width = info.active.width, height = info.active.height }
  end
  if #list == 0 then
    return nil
  end
  return list
end

--- Everything that is written from a GUI event: screens (unwatched) and the
--- engine record (state.json). Safe to call repeatedly; each is written only
--- when it changed. Returns { screens = bool, engine = bool }.
function M.record_environment(dirs, engine_info)
  local list = M.current_screens()
  return {
    screens = list ~= nil and M.write_screens(dirs, list) or false,
    engine = (M.record_engine(dirs, engine_info)) and true or false,
  }
end

return M
