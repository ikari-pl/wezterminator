-- Memory pressure: normal, warning or critical.
-- Needs the stats cache (`pressure=` 1, 2 or 4; `na` where the OS has none).

local common = require 'wzt.segments.common'

return {
  id = 'memory_pressure',
  side = 'right',
  needs_stats = true,
  render = function(ctx)
    local level = ctx.stats and tonumber(ctx.stats.pressure)
    if not level then
      return nil -- absent, or `na`
    end
    local tone, label = 'ok', 'ok'
    if level >= 4 then
      tone, label = 'bad', 'crit'
    elseif level >= 2 then
      tone, label = 'warn', 'warn'
    end
    return { icon = common.glyph(0xF04C5), text = 'MP ' .. label, tone = tone }
  end,
}
