-- Helpers shared by the status segments. Pure Lua: no `wezterm` calls, so the
-- sparkline and parsing rules run under stock Lua in the tests.
--
-- A SEGMENT is a module in this directory returning
--
--   {
--     id          = 'load',            -- one of preset.schema.json status segments
--     side        = 'right',           -- 'left' or 'right' status area
--     needs_stats = true,              -- hides when the stats cache is absent
--     render      = function(ctx) ... end,
--   }
--
-- `render` returns nil (hidden) or an item:
--
--   { icon = <glyph>, text = <string>, tone = <semantic palette key>,
--     spark = <sparkline string, sparkline style only> }
--
-- Colours are SEMANTIC (`ok`, `warn`, `bad`, `accent`, `accent_alt`, `info`,
-- `fg`, `fg_dim`); status.lua turns them into hex through the preset's
-- palette.ui. No segment knows a colour.
--
-- ctx = { stats = table|nil, history = {load = {...}, mem = {...}},
--         window, pane, font = <family>|nil, now = <epoch seconds> }

local M = {}

--- Samples kept per sparkline, and so its fixed width in cells.
M.SAMPLES = 16

M.BLOCKS = { '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█' }

--- Encode a code point as UTF-8 without the `\u{}` escape (absent in Lua 5.1
--- and LuaJIT, which the tests may run on).
function M.glyph(cp)
  if cp < 0x80 then
    return string.char(cp)
  elseif cp < 0x800 then
    return string.char(0xC0 + math.floor(cp / 0x40), 0x80 + cp % 0x40)
  elseif cp < 0x10000 then
    return string.char(
      0xE0 + math.floor(cp / 0x1000),
      0x80 + math.floor(cp / 0x40) % 0x40,
      0x80 + cp % 0x40)
  end
  return string.char(
    0xF0 + math.floor(cp / 0x40000),
    0x80 + math.floor(cp / 0x1000) % 0x40,
    0x80 + math.floor(cp / 0x40) % 0x40,
    0x80 + cp % 0x40)
end

--- Number of characters (not bytes) in a UTF-8 string.
function M.width(s)
  local _, n = s:gsub('[^\128-\191]', '')
  return n
end

--- Append `value` to a ring buffer kept to `cap` samples. Returns the buffer.
function M.push(buf, value, cap)
  cap = cap or M.SAMPLES
  buf[#buf + 1] = value
  while #buf > cap do
    table.remove(buf, 1)
  end
  return buf
end

--- Render a buffer as block-element bars, right-aligned and padded with
--- spaces to a FIXED width, so the status bar does not jitter while the
--- buffer fills.
---
--- The bars autoscale to the min/max observed in the window rather than to an
--- absolute capacity. Scaling load against 24 cores, or memory against 128 GB,
--- collapses every sample in a 16-second window onto one block and the
--- sparkline degenerates into a solid rectangle. Variation is what a
--- sparkline is for; the absolute level is printed next to it.
function M.sparkline(buf, width)
  width = width or M.SAMPLES
  buf = buf or {}
  local first = math.max(1, #buf - width + 1)
  local out = {}
  for _ = 1, width - (#buf - first + 1) do
    out[#out + 1] = ' '
  end
  if #buf == 0 then
    return table.concat(out)
  end

  local lo, hi = buf[first], buf[first]
  for i = first + 1, #buf do
    lo = math.min(lo, buf[i])
    hi = math.max(hi, buf[i])
  end
  local span = hi - lo

  -- A dead-flat series has no shape. Draw it mid-height, which reads as
  -- "steady", rather than on the floor, which reads as "zero".
  local flat = span < 1e-6
  for i = first, #buf do
    local t = flat and 0.5 or ((buf[i] - lo) / span)
    t = math.max(0, math.min(1, t))
    out[#out + 1] = M.BLOCKS[math.floor(t * (#M.BLOCKS - 1) + 0.5) + 1]
  end
  return table.concat(out)
end

--- Tone for a 0..1 utilisation ratio.
function M.heat(ratio)
  if ratio < 0.5 then
    return 'accent'
  elseif ratio < 0.8 then
    return 'warn'
  end
  return 'bad'
end

--- One VPN-style indicator from a cache value: `up`, `wait` or `off`. A key
--- missing from the cache hides the indicator (AE7); a probe that ran and
--- found nothing shows dimmed.
function M.vpn_item(icon, label, value)
  if value == nil then
    return nil
  end
  local tone = 'fg_dim'
  if value == 'up' then
    tone = 'ok'
  elseif value == 'wait' then
    tone = 'warn'
  end
  return { icon = icon, text = label, tone = tone }
end

--- ~-abbreviate a path and keep its last two components when it is long.
function M.short_path(path, home, max)
  max = max or 34
  if type(path) ~= 'string' or path == '' then
    return nil
  end
  if home and home ~= '' and path:sub(1, #home) == home
    and (#path == #home or path:sub(#home + 1, #home + 1) == '/') then
    path = '~' .. path:sub(#home + 1)
  end
  if M.width(path) > max then
    path = '…/' .. (path:match('([^/]+/[^/]+)$') or path)
  end
  return path
end

return M
