-- Battery charge. Lua-native ONLY: it reads wezterm.battery_info() and never
-- the stats cache (the collector deliberately does not probe batteries).
-- Hidden on machines without a battery.

local wezterm = require 'wezterm'
local common = require 'wzt.segments.common'

return {
  id = 'battery',
  side = 'right',
  render = function()
    if type(wezterm.battery_info) ~= 'function' then
      return nil
    end
    local ok, list = pcall(wezterm.battery_info)
    if not ok or type(list) ~= 'table' or #list == 0 then
      return nil
    end
    local battery = list[1]
    local charge = tonumber(battery.state_of_charge)
    if not charge then
      return nil
    end
    local percent = math.floor(charge * 100 + 0.5)
    local tone = 'ok'
    if percent <= 20 then
      tone = 'bad'
    elseif percent <= 40 then
      tone = 'warn'
    end
    local charging = battery.state == 'Charging'
    return {
      icon = common.glyph(charging and 0xF0084 or 0xF0079),
      text = percent .. '%',
      tone = tone,
    }
  end,
}
