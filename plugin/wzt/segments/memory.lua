-- Memory in use over installed RAM, with a sparkline in the sparkline style.
-- Needs the stats cache (`memused=`, `memtotal=`, both in GB).

local common = require 'wzt.segments.common'

return {
  id = 'memory',
  side = 'right',
  needs_stats = true,
  render = function(ctx)
    local stats = ctx.stats
    local used = stats and tonumber(stats.memused)
    local total = stats and tonumber(stats.memtotal)
    if not used or not total or total <= 0 then
      return nil
    end
    return {
      icon = common.glyph(0xF061A),
      text = string.format('%.0f/%.0fG', used, total),
      tone = common.heat(used / total),
      spark = common.sparkline(ctx.history and ctx.history.mem),
    }
  end,
}
