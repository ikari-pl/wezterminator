-- Wall-clock time (HH:MM). Lua-native.

local wezterm = require 'wezterm'
local common = require 'wzt.segments.common'

return {
  id = 'clock',
  side = 'right',
  render = function(ctx)
    local text
    if type(wezterm.strftime) == 'function' then
      local ok, v = pcall(wezterm.strftime, '%H:%M')
      text = ok and v or nil
    end
    text = text or os.date('%H:%M', ctx.now)
    return { icon = common.glyph(0xF017), text = text, tone = 'accent' }
  end,
}
