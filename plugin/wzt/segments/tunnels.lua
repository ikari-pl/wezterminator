-- Tunnels the collector could not name (WireGuard, a corporate VPN, ...), as a
-- count. Shown only when there is at least one. Needs `utun=`.

return {
  id = 'tunnels',
  side = 'right',
  needs_stats = true,
  render = function(ctx)
    local n = ctx.stats and tonumber(ctx.stats.utun)
    if not n or n <= 0 then
      return nil
    end
    return { text = string.format('+%d', n), tone = 'ok' }
  end,
}
