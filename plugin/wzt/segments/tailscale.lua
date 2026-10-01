-- Tailscale state from the stats cache key `ts=up|wait|off`. Hidden when the
-- key is missing, which is what a disabled or unavailable probe produces.

local common = require 'wzt.segments.common'

return {
  id = 'tailscale',
  side = 'right',
  needs_stats = true,
  render = function(ctx)
    return common.vpn_item(common.glyph(0xF0582), 'TS', ctx.stats and ctx.stats.ts)
  end,
}
