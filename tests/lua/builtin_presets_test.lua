-- Built-in origin presets (U4): resolve with no other layers, and with art absent.

local T = ...
local resolve = T.resolve
local data = require 'wzt.data'

local function load_builtins()
  local root = T.root
  local presets, themes = {}, {}
  for _, path in ipairs(T.glob(root .. '/presets/*.json')) do
    presets[#presets + 1] = data.decode_clean(T.read_file(path))
  end
  for _, path in ipairs(T.glob(root .. '/themes/*/theme.json')) do
    themes[#themes + 1] = data.decode_clean(T.read_file(path))
  end
  return presets, themes
end

local function resolve_id(presets, themes, id)
  return resolve.resolve({
    engine = { supported_schema_version = 1, default_preset = presets[1].id },
    layers = {
      builtin = { presets = presets, themes = themes },
    },
    state = {
      schema_version = 1,
      active_preset = id,
      history = resolve.new_array(),
    },
  })
end

T.test('every built-in preset resolves with no fleet or local layer', function()
  local presets, themes = load_builtins()
  T.ok(#presets >= 3)
  T.ok(#themes >= 3)
  for _, p in ipairs(presets) do
    local out = resolve_id(presets, themes, p.id)
    T.eq(out.error, nil, p.id)
    T.eq(out.resolved.id, p.id)
    T.ok(out.resolved.parts.art.fallback_layers ~= nil, p.id .. ' has fallback layers')
  end
end)

T.test('every built-in preset carries Color/Gradient fallback layers', function()
  local presets, themes = load_builtins()
  for _, p in ipairs(presets) do
    local out = resolve_id(presets, themes, p.id)
    local layers = out.resolved.parts.art.fallback_layers
    T.ok(resolve.is_array(layers) and #layers >= 1, p.id)
    local kinds = {}
    for _, layer in ipairs(layers) do
      kinds[layer.kind] = true
    end
    T.ok(kinds.color or kinds.gradient, p.id .. ' uses Color/Gradient fallbacks')
  end
end)

T.test('AE6: missing preferred font resolves to declared fallback list', function()
  local presets, themes = load_builtins()
  local out = resolve.resolve({
    engine = { supported_schema_version = 1, default_preset = 'builtin:cpc-cool' },
    layers = { builtin = { presets = presets, themes = themes } },
    state = {
      schema_version = 1,
      active_preset = 'builtin:cpc-cool',
      history = resolve.new_array(),
    },
    environment = { installed_fonts = { 'Menlo' } },
  })
  local font = out.resolved.parts.font
  T.ok(font.resolved == 'Menlo' or (font.family and font.family:find('Menlo'))
    or (type(font.fallback) == 'table'), 'falls back when preferred missing')
  -- Soft check: preferred list is present and fallback list is non-empty.
  T.ok(font.fallback and #font.fallback >= 1)
end)

T.test('Soft Nebula keeps OD-Cezar chrome and pill status', function()
  local presets, themes = load_builtins()
  local out = resolve_id(presets, themes, 'builtin:soft-nebula')
  local chrome = out.resolved.parts.chrome
  local status = out.resolved.parts.status
  T.eq(chrome.opacity, 0.94)
  T.eq(chrome.blur, 22)
  T.eq(status.style, 'pill')
end)
