-- Preset resolution. PURE LUA: no `wezterm`, no io, no os. It runs under stock
-- Lua against tests/fixtures/resolution/resolve-*.json, and the Rust twin in
-- crates/wzt-model must agree with it. docs/data-model.md is the contract; when
-- the two disagree the fixtures win.
--
-- Input and output shapes are documented in docs/data-model.md ("Resolution").
--
-- Two representation problems are solved here, because Lua has one table type:
--
--   * JSON null. Absent and null are the same thing in this model (null is only
--     used for `based_on`), so callers hand in tables with nulls already
--     removed, and outputs simply omit the field.
--   * Empty arrays versus empty objects. An empty array REPLACES while an empty
--     object MERGES, so the two must never be confused. Decoders that can tag
--     arrays (the test decoder) set the `__wzt_array` metatable marker. Decoders
--     that cannot (wezterm.json_parse, anything that went through
--     wezterm.GLOBAL) are handled by `normalize`, which restores the marker from
--     the schema: the key names below are always arrays.

local M = {}

M.LAYERS = { 'builtin', 'fleet', 'local' }
local LAYER_INDEX = { builtin = 1, fleet = 2, ['local'] = 3 }

---------------------------------------------------------------------------
-- Arrays, copying, comment stripping
---------------------------------------------------------------------------

local array_mt = { __wzt_array = true }
M.array_mt = array_mt

--- Mark `t` (or a new table) as a JSON array so it survives being empty.
function M.new_array(t)
  return setmetatable(t or {}, array_mt)
end

--- True for marked arrays and for non-empty tables with a [1] element. An
--- unmarked empty table is an object.
function M.is_array(t)
  if type(t) ~= 'table' then
    return false
  end
  local mt = getmetatable(t)
  if mt and mt.__wzt_array then
    return true
  end
  if next(t) == nil then
    return false
  end
  return t[1] ~= nil
end
local is_array = M.is_array

--- Deep copy that keeps the array marker.
local function copy(v)
  if type(v) ~= 'table' then
    return v
  end
  local out = {}
  for k, x in pairs(v) do
    out[k] = copy(x)
  end
  local mt = getmetatable(v)
  if mt and mt.__wzt_array then
    setmetatable(out, array_mt)
  end
  return out
end
M.copy = copy

-- Keys whose value is always an array in the v1 schemas. Used to restore the
-- empty-array marker when a decoder could not provide it.
local ARRAY_KEYS = {
  preferred = true, fallback = true, segments = true, layers = true,
  fallback_layers = true, ansi = true, brights = true, hosts = true,
  args = true, project_roots = true, vpn_probes = true,
  screen_overrides = true, screens = true, history = true,
  effective = true, missing = true, catalog = true, ignored = true,
  notices = true, overruled = true, owned_keys = true,
  presets = true, themes = true,
}

-- Maps whose child KEYS are user-chosen names (font families, layer ids,
-- push target names). Those names must never be mistaken for schema keys.
local KEYED_MAPS = { corrections = true, layer_tweaks = true, push_targets = true }

local function is_comment_key(k)
  return type(k) == 'string' and k:sub(1, 1) == '_'
end

-- `key` is the schema key of `v` ('segments', 'layers', ...) or nil when
-- unknown. `opaque` is true inside free-form `params`.
local function walk(v, key, opaque, keep_comments)
  local t = type(v)
  if t ~= 'table' then
    -- A JSON null that survived decoding (mlua light userdata, say) is just
    -- "absent" in this model.
    if t == 'userdata' or t == 'function' or t == 'thread' then
      return nil
    end
    return v
  end

  local out = {}
  local arr = is_array(v)
  if arr then
    for i = 1, #v do
      local c = walk(v[i], nil, opaque, keep_comments)
      if c ~= nil then
        out[#out + 1] = c
      end
    end
  else
    local child_opaque = opaque or key == 'params'
    for k, x in pairs(v) do
      if keep_comments or not is_comment_key(k) then
        local child_key = k
        if KEYED_MAPS[key] then
          child_key = nil
        end
        local c = walk(x, child_key, child_opaque, keep_comments)
        if c ~= nil then
          out[k] = c
        end
      end
    end
  end

  if next(out) == nil then
    local inferred = (not opaque) and key ~= nil and ARRAY_KEYS[key]
    if arr or inferred then
      setmetatable(out, array_mt)
    end
  end
  return out
end

--- Remove every `_`-prefixed object key at every depth, drop null-ish values,
--- and restore empty-array markers. Returns a new tree; the input is untouched.
function M.strip_comments(v)
  return walk(v, nil, false, false)
end

--- Like strip_comments but KEEPS comment keys. Used when a document is read
--- only to be written back (state.json), so hand-written notes survive.
function M.normalize(v, key)
  return walk(v, key, false, true)
end

--- Strip with an explicit schema key for the root (e.g. 'screens').
function M.strip_comments_as(v, key)
  return walk(v, key, false, false)
end

---------------------------------------------------------------------------
-- Deep equality (used by the override aggregator and the tests)
---------------------------------------------------------------------------

local function deep_equal(a, b)
  if a == b then
    return true
  end
  if type(a) ~= 'table' or type(b) ~= 'table' then
    return false
  end
  if next(a) == nil and next(b) == nil then
    return is_array(a) == is_array(b)
  end
  for k, v in pairs(a) do
    if not deep_equal(v, b[k]) then
      return false
    end
  end
  for k in pairs(b) do
    if a[k] == nil then
      return false
    end
  end
  return true
end
M.deep_equal = deep_equal

---------------------------------------------------------------------------
-- Merge
---------------------------------------------------------------------------

-- merge(base, over): objects merge key by key; anything else replaces. An
-- empty array replaces (clears the list); an empty object merges as a no-op
-- over an object and creates an empty object where nothing existed.
local function merge(base, over)
  if type(over) ~= 'table' then
    return over
  end
  if is_array(over) then
    return copy(over)
  end
  if type(base) ~= 'table' or is_array(base) then
    return copy(over)
  end
  local out = copy(base)
  for k, v in pairs(over) do
    out[k] = merge(base[k], v)
  end
  return out
end
M.merge = merge

-- Parts merge key by key, except `scheme`, which is atomic: a layer that sets
-- it replaces the whole part so {theme} and {wezterm_scheme} never combine.
local function merge_parts(base, over)
  local out = copy(base)
  for k, v in pairs(over) do
    if k == 'scheme' then
      out.scheme = copy(v)
    else
      out[k] = merge(base[k], v)
    end
  end
  return out
end

---------------------------------------------------------------------------
-- Add-on ownership
---------------------------------------------------------------------------

-- Table order matters: when several owned keys map to one path, the first in
-- this order is the one reported.
M.ADDON_RULES = {
  { 'font', { 'font' } },
  { 'font_size', { 'font' } },
  { 'font_rules', { 'font' } },
  { 'color_scheme', { 'scheme' } },
  { 'colors', { 'scheme', 'palette' } },
  { 'window_frame', { 'palette' } },
  { 'background', { 'art' } },
  { 'window_background_opacity', { 'chrome.opacity' } },
  { 'macos_window_background_blur', { 'chrome.blur' } },
  { 'kde_window_background_blur', { 'chrome.blur' } },
  -- Not in the data-model table: the nightly Wayland key the platform probe
  -- looks for. Same meaning as the KDE key, so it overrules the same path.
  { 'wayland_window_background_blur', { 'chrome.blur' } },
  { 'window_padding', { 'chrome.padding' } },
  { 'inactive_pane_hsb', { 'chrome.inactive_pane' } },
  { 'enable_tab_bar', { 'chrome.tab_bar.hidden' } },
  { 'tab_bar_at_bottom', { 'chrome.tab_bar.position' } },
}

local function split_path(path)
  local out = {}
  for seg in path:gmatch('[^.]+') do
    out[#out + 1] = seg
  end
  return out
end

-- Remove `path` from `parts` if present. Empty parents are left in place.
local function remove_path(parts, path)
  local segs = split_path(path)
  local node = parts
  for i = 1, #segs - 1 do
    node = type(node) == 'table' and node[segs[i]] or nil
    if type(node) ~= 'table' then
      return false
    end
  end
  local last = segs[#segs]
  if type(node) == 'table' and node[last] ~= nil then
    node[last] = nil
    return true
  end
  return false
end

--- Drop the parts the user's config owns. Returns the `overruled` list.
function M.drop_owned(parts, owned_keys)
  local owned = {}
  for _, k in ipairs(owned_keys or {}) do
    owned[k] = true
  end
  local reported, list = {}, {}
  for _, rule in ipairs(M.ADDON_RULES) do
    if owned[rule[1]] then
      for _, path in ipairs(rule[2]) do
        if not reported[path] and remove_path(parts, path) then
          reported[path] = true
          list[#list + 1] = { path = path, config_key = rule[1] }
        end
      end
    end
  end
  table.sort(list, function(a, b)
    return a.path < b.path
  end)
  return list
end

---------------------------------------------------------------------------
-- Resolution
---------------------------------------------------------------------------

-- Returns ok, reason, found.
local function gate(doc, supported)
  if type(doc) ~= 'table' then
    return false, 'invalid_document', nil
  end
  local v = doc.schema_version
  if type(v) ~= 'number' or v ~= v or v ~= math.floor(v) or v < 1 then
    return false, 'invalid_schema_version', v
  end
  if v > supported then
    return false, 'unsupported_schema_version', v
  end
  return true
end

local function name_key(name)
  if type(name) ~= 'string' then
    return ''
  end
  return (name:gsub('^%s+', ''):gsub('%s+$', ''):lower())
end

local function compute_font(font, installed)
  local out = copy(font)
  local have
  if installed ~= nil then
    have = {}
    for _, f in ipairs(installed) do
      have[f] = true
    end
  end
  local effective, seen, missing = {}, {}, {}
  for _, f in ipairs(font.preferred or {}) do
    if have == nil or have[f] then
      if not seen[f] then
        seen[f] = true
        effective[#effective + 1] = f
      end
    else
      missing[#missing + 1] = f
    end
  end
  for _, f in ipairs(font.fallback or {}) do
    if not seen[f] then
      seen[f] = true
      effective[#effective + 1] = f
    end
  end
  out.effective = M.new_array(effective)
  out.missing = M.new_array(missing)
  return out
end

--- Resolve the active preset. See docs/data-model.md.
function M.resolve(input)
  local engine = input.engine or {}
  local supported = engine.supported_schema_version or 1
  local default_id = engine.default_preset
  local layers_in = input.layers or {}
  local env = input.environment or {}

  local ignored = {}
  local function ignore(layer, kind, ref, reason, found)
    local e = { kind = kind, ref = ref, reason = reason }
    if layer then
      e.layer = layer
    end
    if found ~= nil then
      e.found = found
    end
    ignored[#ignored + 1] = e
  end

  -- Steps 1-3: strip, gate versions, check identity. ----------------------
  local presets = {} -- accepted entries {id, name, layer, doc}
  local themes = {} -- id -> theme document
  local overrides, machines = {}, {}

  for _, lname in ipairs(M.LAYERS) do
    local L = layers_in[lname]
    if type(L) == 'table' then
      local function take(list, kind, sink)
        local seen = {}
        for _, raw in ipairs(list or {}) do
          local ok, reason, found = gate(raw, supported)
          local id = type(raw) == 'table' and type(raw.id) == 'string' and raw.id or nil
          if not ok then
            ignore(lname, kind, id or '<unknown>', reason, found)
          else
            local ns = id and id:match('^([^:]+):')
            if ns ~= lname then
              ignore(lname, kind, id or '<unknown>', 'id_layer_mismatch')
            elseif seen[id] then
              ignore(lname, kind, id, 'duplicate_id')
            else
              seen[id] = true
              sink(id, M.strip_comments(raw))
            end
          end
        end
      end

      take(L.presets, 'preset', function(id, doc)
        presets[#presets + 1] = { id = id, name = doc.name, layer = lname, doc = doc }
      end)
      take(L.themes, 'theme', function(id, doc)
        themes[id] = doc
      end)

      -- The built-in layer never ships overrides or machine settings, and the
      -- loader does not read them even if supplied.
      if lname ~= 'builtin' then
        local function single(raw, kind, ref, sink)
          if raw == nil then
            return
          end
          local ok, reason, found = gate(raw, supported)
          if not ok then
            ignore(lname, kind, ref, reason, found)
          else
            sink(M.strip_comments(raw))
          end
        end
        single(L.overrides, 'overrides', 'overrides', function(d)
          overrides[lname] = d
        end)
        single(L.machine, 'machine', 'machine', function(d)
          machines[lname] = d
        end)
      end
    end
  end

  -- The state document is gated last.
  local state = input.state
  if state ~= nil then
    local ok, reason, found = gate(state, supported)
    if not ok then
      ignore(nil, 'state', 'state', reason, found)
      state = nil
    else
      state = M.strip_comments(state)
    end
  end

  -- Step 4: catalog. -------------------------------------------------------
  table.sort(presets, function(a, b)
    local la, lb = LAYER_INDEX[a.layer], LAYER_INDEX[b.layer]
    if la ~= lb then
      return la < lb
    end
    return a.id < b.id
  end)

  local by_id = {}
  local top = {} -- name key -> { layer = idx, id = lowest id in that layer }
  for _, p in ipairs(presets) do
    by_id[p.id] = p
    local nk = name_key(p.name)
    local li = LAYER_INDEX[p.layer]
    local t = top[nk]
    if not t or li > t.layer then
      top[nk] = { layer = li, id = p.id }
    end
  end

  local catalog = {}
  for _, p in ipairs(presets) do
    local t = top[name_key(p.name)]
    local shadowed_by
    if t.layer > LAYER_INDEX[p.layer] then
      shadowed_by = t.id
    end
    catalog[#catalog + 1] = { id = p.id, name = p.name, layer = p.layer, shadowed_by = shadowed_by }
  end

  -- Step 5: pick candidates. ----------------------------------------------
  local notices = {}
  local requested = state and type(state.active_preset) == 'string' and state.active_preset or nil
  local candidates
  if requested == nil then
    notices[#notices + 1] = { code = 'no_active_preset', used = default_id }
    candidates = { default_id }
  elseif not by_id[requested] then
    notices[#notices + 1] = { code = 'active_preset_missing', requested = requested, used = default_id }
    candidates = { default_id }
  elseif requested == default_id then
    candidates = { requested }
  else
    candidates = { requested, default_id }
  end

  -- Steps 6-7: overrides and theme expansion. ------------------------------
  local function build(preset)
    local parts = copy(preset.doc.parts or {})
    for _, ln in ipairs({ 'fleet', 'local' }) do
      local o = overrides[ln]
      if o and type(o.parts) == 'table' then
        parts = merge_parts(parts, o.parts)
      end
    end

    local art_id = parts.art and parts.art.theme
    local pal_id = parts.palette and parts.palette.theme
    local sch_id = parts.scheme and parts.scheme.theme
    for _, id in ipairs({ art_id or false, pal_id or false, sch_id or false }) do
      if id and not themes[id] then
        return nil, { code = 'theme_missing', preset = preset.id, theme = id }
      end
    end

    local out = {}
    local art_theme = art_id and themes[art_id]

    if parts.art and art_theme then
      local tart = art_theme.art or {}
      local tweaks = parts.art.layer_tweaks or {}
      local layers = {}
      for _, layer in ipairs(tart.layers or {}) do
        local tw = type(layer.id) == 'string' and tweaks[layer.id]
        layers[#layers + 1] = tw and merge(layer, tw) or copy(layer)
      end
      out.art = {
        theme = art_id,
        seed = tart.seed,
        base_color = tart.base_color,
        layers = M.new_array(layers),
        fallback_layers = copy(art_theme.fallback_layers) or M.new_array(),
      }
    end

    if parts.scheme then
      if sch_id then
        out.scheme = { theme = sch_id, colors = copy(themes[sch_id].palette and themes[sch_id].palette.scheme) }
      else
        out.scheme = copy(parts.scheme)
      end
    end

    if parts.palette and pal_id then
      out.palette = { theme = pal_id, ui = copy(themes[pal_id].palette and themes[pal_id].palette.ui) }
    end

    if art_theme or parts.motion then
      out.motion = merge(copy(art_theme and art_theme.motion or {}), parts.motion or {})
    end

    if parts.font then
      out.font = compute_font(parts.font, env.installed_fonts)
    end
    out.chrome = copy(parts.chrome)
    out.status = copy(parts.status)

    return {
      id = preset.id,
      name = preset.name,
      based_on = preset.doc.based_on,
      parts = out,
    }
  end

  local resolved
  for _, cid in ipairs(candidates) do
    local p = cid and by_id[cid]
    if p then
      local r, notice = build(p)
      if r then
        resolved = r
        break
      end
      notices[#notices + 1] = notice
    end
  end

  -- Step 8: add-on ownership. ---------------------------------------------
  local overruled = {}
  if resolved and input.addon then
    overruled = M.drop_owned(resolved.parts, input.addon.owned_keys)
  end

  -- Step 9: machine settings. ---------------------------------------------
  local machine = {}
  for _, ln in ipairs({ 'fleet', 'local' }) do
    local m = machines[ln]
    if m then
      local d = copy(m)
      d.schema_version = nil
      machine = merge(machine, d)
    end
  end

  local active = { requested = requested, id = resolved and resolved.id or nil }
  active.fell_back = active.id ~= nil and active.id ~= requested

  return {
    active = active,
    catalog = M.new_array(catalog),
    resolved = resolved,
    machine = machine,
    overruled = M.new_array(overruled),
    ignored = M.new_array(ignored),
    notices = M.new_array(notices),
    error = (resolved == nil) and { code = 'no_resolvable_preset' } or nil,
  }
end

return M
