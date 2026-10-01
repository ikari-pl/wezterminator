-- AWS VPN state from the stats cache key `aws=up|wait|off` (the optional AWS
-- probe in U6). Hidden when the key is missing.

local common = require 'wzt.segments.common'

return {
  id = 'aws_vpn',
  side = 'right',
  needs_stats = true,
  render = function(ctx)
    local stats = ctx.stats
    local value = stats and (stats.aws or stats.aws_vpn)
    return common.vpn_item(common.glyph(0xF033E), 'AWS', value)
  end,
}
