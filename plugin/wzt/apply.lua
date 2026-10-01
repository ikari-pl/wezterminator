-- Turn a resolved preset into WezTerm config. Resolution is pure data; this is
-- where defaults live (an absent opacity means 1, an absent `enabled` means
-- true) and where the platform's limits are applied.
--
-- `fragment` builds the config keys. `background` builds the layer stack once,
-- as plain data, together with the parallax factor of every layer. The same
-- data serves startup and live updates: the override aggregator calls
-- `compose_background` with offsets, so the two paths cannot drift (the lesson
-- of metis's backgrounds.lua `config_for`).
--
-- Everything returned is plain data (strings, numbers, tables) so it can be
-- kept in wezterm.GLOBAL. Functions and userdata cannot.

local wezterm = require 'wezterm'
local platform = require 'wzt.platform'

local M = {}

--- How far a layer may drift from its resting position, in pixels. Bounded so
--- layers cannot slide off their artwork (metis parallax.lua uses 620).
M.SLACK = 620

M.FONT_SIZE_MIN = 6.0
M.FONT_SIZE_MAX = 40.0
M.FONT_SIZE_DEFAULT = 12.0 -- WezTerm's own default

local function clamp(v, limit)
  return math.max(-limit, math.min(limit, v))
end

local function copy(v)
  if type(v) ~= 'table' then
    return v
  end
  local out = {}
  for k, x in pairs(v) do
    out[k] = copy(x)
  end
  return out
end

---------------------------------------------------------------------------
-- Colours
---------------------------------------------------------------------------

local SCHEME_KEYS = {
  'foreground', 'background', 'cursor_bg', 'cursor_fg', 'cursor_border',
  'selection_bg', 'selection_fg', 'ansi', 'brights',
}

local function scheme_colors(scheme)
  local colors = {}
  for _, k in ipairs(SCHEME_KEYS) do
    if scheme[k] ~= nil then
      colors[k] = copy(scheme[k])
    end
  end
  return colors
end

local function tab_bar(ui)
  local hover_bg = ui.tab_hover_bg or ui.tab_active_bg
  local hover_fg = ui.tab_hover_fg or ui.tab_active_fg
  return {
    background = ui.tab_bar_bg,
    active_tab = { bg_color = ui.tab_active_bg, fg_color = ui.tab_active_fg },
    inactive_tab = { bg_color = ui.tab_inactive_bg, fg_color = ui.tab_inactive_fg },
    inactive_tab_hover = { bg_color = hover_bg, fg_color = hover_fg },
    new_tab = { bg_color = ui.tab_inactive_bg, fg_color = ui.tab_inactive_fg },
    new_tab_hover = { bg_color = hover_bg, fg_color = hover_fg },
  }
end

local function window_frame(ui)
  return {
    active_titlebar_bg = ui.tab_bar_bg,
    inactive_titlebar_bg = ui.tab_bar_bg,
    active_titlebar_fg = ui.tab_active_fg,
    inactive_titlebar_fg = ui.tab_inactive_fg,
    button_fg = ui.tab_inactive_fg,
    button_bg = ui.tab_bar_bg,
    button_hover_fg = ui.tab_active_fg,
    button_hover_bg = ui.tab_active_bg,
  }
end

---------------------------------------------------------------------------
-- Fonts
---------------------------------------------------------------------------

--- Font size is BASE + PER-FONT CORRECTION, not an absolute size per font: a
--- bitmap face and a vector face at the same nominal size look wildly
--- different, so each font carries a relative correction and the base rescales
--- everything at once (metis themes.lua). Returns nil when the preset says
--- nothing about size at all.
function M.font_size(font)
  if not font then
    return nil
  end
  local primary = font.effective and font.effective[1]
  local delta = primary and font.corrections and font.corrections[primary]
  if font.size == nil and delta == nil then
    return nil
  end
  local size = (font.size or M.FONT_SIZE_DEFAULT) + (delta or 0)
  return math.max(M.FONT_SIZE_MIN, math.min(M.FONT_SIZE_MAX, size))
end

---------------------------------------------------------------------------
-- Config fragment
---------------------------------------------------------------------------

--- Build the config keys for a resolved preset.
---
--- `host` is platform.info(). Returns fragment, unavailable, where
--- `unavailable` lists chrome options this platform cannot honour:
--- { {path = 'chrome.blur', reason = '...'}, ... }. Those are reported, never set.
function M.fragment(resolved, host)
  local parts = resolved.parts
  local cfg, unavailable = {}, {}

  local scheme = parts.scheme
  if scheme then
    if scheme.wezterm_scheme then
      cfg.color_scheme = scheme.wezterm_scheme
      -- A stock scheme brings its own background, which can sit far from the
      -- art (Gruvbox is luminance 27-48 against Ember's 8). The art's base
      -- colour wins so the scheme cannot fight the artwork (metis
      -- backgrounds.lua, `config_for`).
      local base = parts.art and parts.art.base_color
      if base then
        cfg.colors = { background = base }
      end
    elseif scheme.colors then
      cfg.colors = scheme_colors(scheme.colors)
    end
  end

  local palette = parts.palette
  if palette and palette.ui then
    cfg.colors = cfg.colors or {}
    cfg.colors.tab_bar = tab_bar(palette.ui)
    cfg.window_frame = window_frame(palette.ui)
  end

  local font = parts.font
  if font then
    if font.effective and #font.effective > 0 then
      cfg.font = wezterm.font_with_fallback(copy(font.effective))
    end
    local size = M.font_size(font)
    if size then
      cfg.font_size = size
    end
  end

  local chrome = parts.chrome
  if chrome then
    if chrome.opacity ~= nil then
      cfg.window_background_opacity = chrome.opacity
    end
    if chrome.blur ~= nil and chrome.blur > 0 then
      if host.blur_key then
        -- macOS takes a radius; the KDE and Wayland keys are plain on/off.
        if host.blur_key == 'macos_window_background_blur' then
          cfg[host.blur_key] = chrome.blur
        else
          cfg[host.blur_key] = true
        end
      else
        unavailable[#unavailable + 1] = {
          path = 'chrome.blur',
          reason = host.blur_reason or 'window blur is not available here',
        }
      end
    end
    if chrome.padding then
      cfg.window_padding = copy(chrome.padding)
    end
    if chrome.inactive_pane then
      cfg.inactive_pane_hsb = {
        saturation = chrome.inactive_pane.saturation or 1.0,
        brightness = chrome.inactive_pane.brightness or 1.0,
      }
    end
    local tb = chrome.tab_bar
    if tb then
      if tb.hidden ~= nil then
        cfg.enable_tab_bar = not tb.hidden
      end
      if tb.position ~= nil then
        cfg.tab_bar_at_bottom = tb.position == 'bottom'
      end
    end
  end

  return cfg, unavailable
end

---------------------------------------------------------------------------
-- Background art
---------------------------------------------------------------------------

local function base_layer(color)
  return { source = { Color = color }, width = '100%', height = '100%', opacity = 1.0 }
end

local function fallback_to_layer(fb)
  local opacity = fb.opacity or 1.0
  if fb.kind == 'color' then
    return { source = { Color = fb.color }, width = '100%', height = '100%', opacity = opacity }
  end
  local orientation
  if fb.shape == 'radial' then
    orientation = { Radial = { cx = 0.5, cy = 0.5, radius = 0.5 } }
  else
    orientation = { Linear = { angle = fb.angle or 0.0 } }
  end
  return {
    source = { Gradient = { colors = copy(fb.colors), orientation = orientation } },
    width = '100%',
    height = '100%',
    opacity = opacity,
  }
end

local function namespace_dir(ns, where)
  if ns == 'builtin' then
    return where.builtin
  elseif ns == 'fleet' then
    return where.dirs.fleet
  end
  return where.dirs['local']
end

--- The resolution art should be found at: recorded screens (largest pixel
--- area wins), each optionally replaced by a machine `screen_overrides`
--- entry; when nothing was recorded, the overrides alone. nil when unknown.
function M.pick_resolution(screens, overrides)
  local candidates = {}
  for _, s in ipairs(screens or {}) do
    local w, h = s.width, s.height
    for _, o in ipairs(overrides or {}) do
      if o.name == nil or o.name == s.name then
        w, h = o.width, o.height
        break
      end
    end
    candidates[#candidates + 1] = { w = w, h = h }
  end
  if #candidates == 0 then
    for _, o in ipairs(overrides or {}) do
      candidates[#candidates + 1] = { w = o.width, h = o.height }
    end
  end
  local best
  for _, c in ipairs(candidates) do
    if not best or c.w * c.h > best.w * best.h then
      best = c
    end
  end
  return best
end

-- Art directories in lookup order. Each entry: { kind, dir, resolution }.
local function art_sources(art, ctx)
  local ns, slug = art.theme:match('^([^:]+):(.+)$')
  local list = {}
  if not slug then
    return list
  end
  local res = ctx.resolution
  if ctx.machine and ctx.machine.dev_art_path then
    -- Development only: <dev_art_path>/<theme slug>/<layer id>.png. Consulted
    -- before everything else; doctor flags it.
    list[#list + 1] = { kind = 'dev', dir = ctx.machine.dev_art_path .. '/' .. slug, resolution = res }
  end
  if res then
    local dir = string.format('%dx%d', res.w, res.h)
    -- User-generated art. The recipe-hash manifest check needs blake3, which
    -- Lua does not have; the Rust side writes the art and `doctor` reports a
    -- stale manifest. `ctx.accept_art` is the hook for a stricter check.
    list[#list + 1] = { kind = 'user', dir = ctx.where.dirs.data .. '/art/' .. slug .. '/' .. dir, resolution = res }
    list[#list + 1] = {
      kind = 'shipped',
      dir = namespace_dir(ns, ctx.where) .. '/themes/' .. slug .. '/art/' .. dir,
      resolution = res,
    }
  end
  return list
end

local function layer_enabled(layer)
  return layer.enabled ~= false
end

local function repeat_flags(mode)
  mode = mode or 'none'
  return (mode == 'x' or mode == 'xy') and 'Repeat' or 'NoRepeat',
    (mode == 'y' or mode == 'xy') and 'Repeat' or 'NoRepeat'
end

--- Build the background stack for a resolved `art` part.
---
--- ctx = {
---   where   = { builtin = plugin dir, dirs = platform.dirs() },
---   machine = merged machine settings (for dev_art_path),
---   resolution = {w, h} or nil,
---   default_base = colour used when the theme has no base_color,
---   scrollback_parallax = false pins every layer (motion.scrollback_parallax
---     is off); anything else lets layers use WezTerm's native Parallax,
---   accept_art = optional function(source) -> bool,
--- }
---
--- Returns {
---   layers = WezTerm background layers, bottom to top, base colour first,
---   meta   = per-layer {vertical, horizontal} parallax factors, same indexing,
---   source = 'dev' | 'user' | 'shipped' | 'fallback',
---   resolution = {w, h} or nil,
--- }
function M.background(art, ctx)
  local base = base_layer(art.base_color or ctx.default_base or '#000000')

  local enabled = {}
  for _, layer in ipairs(art.layers or {}) do
    if layer_enabled(layer) then
      enabled[#enabled + 1] = layer
    end
  end

  if #enabled > 0 then
    for _, src in ipairs(art_sources(art, ctx)) do
      local complete = true
      for _, layer in ipairs(enabled) do
        if not platform.file_exists(src.dir .. '/' .. layer.id .. '.png') then
          complete = false
          break
        end
      end
      if complete and (not ctx.accept_art or ctx.accept_art(src)) then
        local layers, meta = { base }, { { vertical = 0, horizontal = 0 } }
        for _, layer in ipairs(enabled) do
          local pv = layer.parallax and layer.parallax.vertical or 0
          local ph = layer.parallax and layer.parallax.horizontal or 0
          local rx, ry = repeat_flags(layer['repeat'])
          local entry = {
            source = { File = src.dir .. '/' .. layer.id .. '.png' },
            horizontal_align = 'Center',
            vertical_align = 'Middle',
            repeat_x = rx,
            repeat_y = ry,
            opacity = layer.opacity or 1.0,
            vertical_offset = 0,
            horizontal_offset = 0,
            attachment = (pv ~= 0 and ctx.scrollback_parallax ~= false) and { Parallax = pv } or 'Fixed',
          }
          -- Pin non-repeating layers to the art's native size; leaving them at
          -- "100%" would resample and smear the hard pixel edges. Repeating
          -- tiles keep WezTerm's default sizing.
          if src.resolution and rx == 'NoRepeat' and ry == 'NoRepeat' then
            entry.width = src.resolution.w
            entry.height = src.resolution.h
          end
          layers[#layers + 1] = entry
          meta[#meta + 1] = { vertical = pv, horizontal = ph }
        end
        return { layers = layers, meta = meta, source = src.kind, resolution = src.resolution }
      end
    end
  end

  local layers, meta = { base }, { { vertical = 0, horizontal = 0 } }
  for _, fb in ipairs(art.fallback_layers or {}) do
    layers[#layers + 1] = fallback_to_layer(fb)
    meta[#meta + 1] = { vertical = 0, horizontal = 0 }
  end
  return { layers = layers, meta = meta, source = 'fallback', resolution = ctx.resolution }
end

--- Apply scroll offsets to a stack from `background`, or pause it.
---
--- `offsets` = { vertical = px, horizontal = px } is a virtual scroll position;
--- each layer moves by that times its own parallax factor, clamped to SLACK.
--- Paused stacks collapse to the base colour (R18); the offsets are not lost
--- because they live in their own channels, not in this result.
function M.compose_background(bg, offsets, paused)
  if paused then
    return { copy(bg.layers[1]) }
  end
  local ov = (offsets and offsets.vertical) or 0
  local oh = (offsets and offsets.horizontal) or 0
  local out = {}
  for i, layer in ipairs(bg.layers) do
    local l = copy(layer)
    local m = bg.meta and bg.meta[i]
    if m then
      if m.vertical ~= 0 and ov ~= 0 then
        l.vertical_offset = (l.vertical_offset or 0) + clamp(ov * m.vertical, M.SLACK)
      end
      if m.horizontal ~= 0 and oh ~= 0 then
        l.horizontal_offset = (l.horizontal_offset or 0) + clamp(oh * m.horizontal, M.SLACK)
      end
    end
    out[#out + 1] = l
  end
  return out
end

--- Convenience for callers that want only the layers at rest.
function M.rest_layers(bg)
  return M.compose_background(bg, nil, false)
end

return M
