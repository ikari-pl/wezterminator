-- The status bar, in two styles:
--
--   sparkline  (from metis)    coloured glyphs and block-element sparklines
--                              separated by thin bars
--   pill       (from OD-Cezar) each segment a filled pill: tinted icon, plain
--                              text, on the palette's surface colour
--
-- WHICH segments, in WHAT order, and in WHICH style come from the resolved
-- preset's `status` part. Colours come from the preset's `palette.ui` through
-- SEMANTIC keys (`ok`, `warn`, `accent`, ...), never raw hex.
--
-- WHERE the data comes from (plan: U3):
--   * Lua-native sources: clock, cwd, battery, workspace, exit code, font.
--   * The stats cache, one `key=value ...` line that `wezterminator stats`
--     writes. A segment that needs it HIDES when the key (or the whole file)
--     is absent, so with the binary missing the bar still works.
--   * Battery is never read from the cache.
--
-- THE GOLDEN RULE of this file: `update-status` runs on the GUI thread, about
-- once a second, for EVERY window. Nothing here waits on a subprocess. It
-- reads the last result from the cache file (microseconds) and starts the next
-- collection fire-and-forget, at most once per second across all windows.
--
-- STATE: every set_config_overrides call re-evaluates the config in a fresh
-- Lua state, and auto-scroll writes once per tick. Sparkline history therefore
-- lives in wezterm.GLOBAL (key `wzt_status`), not in module locals, or it
-- would reset every second.

local wezterm = require 'wezterm'
local platform = require 'wzt.platform'
local common = require 'wzt.segments.common'

local M = {}

local STORE_KEY = 'wzt_status'

M.SAMPLES = common.SAMPLES
M.sparkline = common.sparkline
M.push = common.push

--- The segment ids preset.schema.json allows. Anything else is ignored.
M.KNOWN = {
  'load', 'memory', 'memory_pressure', 'tailscale', 'warp', 'tunnels', 'aws_vpn',
  'cwd', 'font', 'battery', 'exit_code', 'clock', 'workspace',
}

local KNOWN_SET = {}
for _, id in ipairs(M.KNOWN) do
  KNOWN_SET[id] = true
end

-- When a palette lacks a semantic key, fall back to a related one rather than
-- to a raw colour.
local TONE_FALLBACK = {
  info = 'accent',
  accent_alt = 'accent',
  fg_dim = 'fg',
  surface = 'bg',
}

---------------------------------------------------------------------------
-- The stats cache
---------------------------------------------------------------------------

--- Parse one cache line (`load=1.2 memused=40 ts=up`). Returns a table of
--- strings, or nil when the line holds no pairs.
function M.parse_line(line)
  if type(line) ~= 'string' then
    return nil
  end
  local t, n = {}, 0
  for k, v in line:gmatch('([%w_]+)=([^%s]+)') do
    t[k] = v
    n = n + 1
  end
  if n == 0 then
    return nil
  end
  return t
end

--- Read the cache. nil when the file is absent or empty. Only the first line
--- is read; the collector writes exactly one.
function M.read_stats(path)
  local f = path and io.open(path, 'r')
  if not f then
    return nil
  end
  local line = f:read('*l')
  f:close()
  return M.parse_line(line)
end

function M.stats_path(dirs)
  return dirs.state .. '/stats'
end

--- Start `wezterminator stats` without waiting for it. The collector writes
--- the cache atomically itself. Returns true when a process was started.
--- A failed start (no binary) is remembered so it is not retried every second.
function M.collect_async(opts)
  if opts.collect == false or type(wezterm.background_child_process) ~= 'function' then
    return false
  end
  local g = platform.store_get(STORE_KEY) or {}
  if g.collector_failed then
    return false
  end
  local ok = pcall(wezterm.background_child_process, { opts.binary or 'wezterminator', 'stats' })
  if not ok then
    g.collector_failed = true
    platform.store_set(STORE_KEY, g)
  end
  return ok
end

---------------------------------------------------------------------------
-- Segments
---------------------------------------------------------------------------

local loaded = {}

local function load_segment(id)
  if not KNOWN_SET[id] then
    return nil
  end
  if loaded[id] == nil then
    local ok, seg = pcall(require, 'wzt.segments.' .. id)
    loaded[id] = ok and type(seg) == 'table' and seg or false
  end
  return loaded[id] or nil
end

--- Render the listed segments, in order. Returns a list of
--- { id, side, item } for the visible ones. Segments that need the stats cache
--- are skipped when `ctx.stats` is nil, and a segment that fails is skipped
--- rather than taking the bar down.
function M.collect(ids, ctx)
  local out = {}
  for _, id in ipairs(ids or {}) do
    local seg = load_segment(id)
    if seg and not (seg.needs_stats and ctx.stats == nil) then
      local ok, item = pcall(seg.render, ctx)
      if ok and type(item) == 'table' then
        out[#out + 1] = { id = id, side = seg.side or 'right', item = item }
      end
    end
  end
  return out
end

---------------------------------------------------------------------------
-- Formatting
---------------------------------------------------------------------------

local function colour(ui, tone)
  if not ui or not tone then
    return nil
  end
  local seen = {}
  while tone and not seen[tone] do
    seen[tone] = true
    if ui[tone] then
      return ui[tone]
    end
    tone = TONE_FALLBACK[tone]
  end
  return nil
end
M.colour = colour

local function add(list, key, value)
  if value ~= nil then
    list[#list + 1] = { [key] = { Color = value } }
  end
end

local function text(list, s)
  list[#list + 1] = { Text = s }
end

--- Turn rendered segments of one side into wezterm.format elements.
--- style: 'sparkline' or 'pill'. ui: the preset's palette.ui (may be nil).
function M.elements(shown, side, style, ui)
  local list = {}
  local first = true
  for _, s in ipairs(shown) do
    if s.side == side then
      local item = s.item
      local tone = colour(ui, item.tone)
      if style == 'pill' then
        -- ResetAttributes between pills gives the gap its own (bar) background.
        if not first then
          list[#list + 1] = 'ResetAttributes'
          text(list, ' ')
        end
        add(list, 'Background', colour(ui, 'surface'))
        add(list, 'Foreground', tone)
        text(list, ' ' .. (item.icon and (item.icon .. ' ') or ''))
        add(list, 'Foreground', colour(ui, 'fg'))
        text(list, item.text .. ' ')
      else
        if not first then
          add(list, 'Foreground', colour(ui, 'fg_dim'))
          text(list, ' │ ')
        end
        add(list, 'Foreground', tone)
        text(list, ' ' .. (item.icon and (item.icon .. ' ') or '')
          .. (item.spark and (item.spark .. ' ') or '') .. item.text)
      end
      first = false
    end
  end
  if not first then
    if style == 'pill' then
      list[#list + 1] = 'ResetAttributes'
    end
    text(list, ' ')
  end
  return list
end

---------------------------------------------------------------------------
-- The update-status handler
---------------------------------------------------------------------------

--- The sparkline history, sampled at most once per second in total.
--- Returns history, sampled (true when this call took the sample).
local function sample(stats)
  local g = platform.store_get(STORE_KEY) or {}
  local now = platform.now()
  local sampled = false
  if g.tick ~= now then
    g.tick = now
    sampled = true
    if stats then
      g.load = common.push(g.load or {}, tonumber(stats.load) or 0)
      g.mem = common.push(g.mem or {}, tonumber(stats.memused) or 0)
    end
    platform.store_set(STORE_KEY, g)
  end
  return { load = g.load or {}, mem = g.mem or {} }, sampled
end

--- One status refresh for a window. `opts` = {
---   status = { style, segments }   the resolved preset's status part
---   ui = palette.ui or nil
---   font = family or nil
---   stats_path, home, binary, collect = false to never start the collector
--- }
--- Returns { left = elements, right = elements, shown = {ids} } for tests.
function M.update(window, pane, opts)
  local status = opts.status or {}
  local stats = M.read_stats(opts.stats_path)
  local history, sampled = sample(stats)
  if sampled then
    M.collect_async(opts)
  end

  local ctx = {
    stats = stats,
    history = history,
    window = window,
    pane = pane,
    font = opts.font,
    home = opts.home,
    now = platform.now(),
  }
  local shown = M.collect(status.segments, ctx)
  local style = status.style == 'pill' and 'pill' or 'sparkline'
  local left = M.elements(shown, 'left', style, opts.ui)
  local right = M.elements(shown, 'right', style, opts.ui)

  pcall(function()
    window:set_left_status(wezterm.format(left))
    window:set_right_status(wezterm.format(right))
  end)

  local ids = {}
  for _, s in ipairs(shown) do
    ids[#ids + 1] = s.id
  end
  return { left = left, right = right, shown = ids }
end

--- Register the handler. `ctx` = {
---   resolved = the resolved preset (nil: nothing to do)
---   dirs, owned_set, binary, collect, home
--- }
--- Returns a summary, or nil when there is no status part.
function M.setup(config, ctx)
  local resolved = ctx.resolved
  local status = resolved and resolved.parts.status
  if not status then
    return nil
  end
  local font = resolved.parts.font
  local opts = {
    status = status,
    ui = resolved.parts.palette and resolved.parts.palette.ui,
    font = font and font.effective and font.effective[1],
    stats_path = M.stats_path(ctx.dirs),
    home = ctx.home or wezterm.home_dir,
    binary = ctx.binary,
    collect = ctx.collect,
  }

  if not (ctx.owned_set and ctx.owned_set.status_update_interval) then
    config.status_update_interval = 1000
  end

  wezterm.on('update-status', function(window, pane)
    M.update(window, pane, opts)
  end)

  return { style = status.style, segments = status.segments, stats_path = opts.stats_path }
end

return M
