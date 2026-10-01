-- The family of the font in use. Lua-native: status.lua passes it in
-- `ctx.font` from the resolved font part (first effective family).

local common = require 'wzt.segments.common'

-- "FiraCode Nerd Font Mono" -> "FiraCode". The suffixes only describe the
-- patched build, not the design.
local function short(name)
  name = name:gsub('%s+Nerd Font.*$', ''):gsub('%s+Mono$', '')
  return name
end

return {
  id = 'font',
  side = 'right',
  render = function(ctx)
    if type(ctx.font) ~= 'string' or ctx.font == '' then
      return nil
    end
    return { icon = common.glyph(0xF06D6), text = short(ctx.font), tone = 'accent_alt' }
  end,
}
