-- 1-minute load average, with a sparkline in the sparkline style.
-- Needs the stats cache (`load=`, optional `ncpu=`).

local common = require 'wzt.segments.common'

return {
  id = 'load',
  side = 'right',
  needs_stats = true,
  render = function(ctx)
    local stats = ctx.stats
    local load = stats and tonumber(stats.load)
    if not load then
      return nil
    end
    -- Scaled to core count, so "full" means every core busy.
    local ncpu = tonumber(stats.ncpu) or 8
    if ncpu < 1 then
      ncpu = 1
    end
    return {
      icon = common.glyph(0xF035B),
      text = string.format('%.2f', load),
      tone = common.heat(load / ncpu),
      spark = common.sparkline(ctx.history and ctx.history.load),
    }
  end,
}
