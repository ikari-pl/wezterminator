-- Cloudflare WARP state from the stats cache key `warp=up|wait|off`. Hidden
-- when the key is missing (AE7).

local common = require 'wzt.segments.common'

return {
  id = 'warp',
  side = 'right',
  needs_stats = true,
  render = function(ctx)
    return common.vpn_item(common.glyph(0xF015F), 'WARP', ctx.stats and ctx.stats.warp)
  end,
}
