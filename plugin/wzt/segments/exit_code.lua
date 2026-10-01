-- Exit status of the last command, from the `wezterm_exit_code` user var that
-- the shell integration sets. Lua-native. Hidden until the shell reports one.

local common = require 'wzt.segments.common'

return {
  id = 'exit_code',
  side = 'right',
  render = function(ctx)
    local pane = ctx.pane
    if not pane then
      return nil
    end
    local ok, vars = pcall(function()
      return pane:get_user_vars()
    end)
    local code = ok and type(vars) == 'table' and vars.wezterm_exit_code or nil
    if code == nil or code == '' then
      return nil
    end
    local succeeded = tonumber(code) == 0
    return {
      icon = common.glyph(succeeded and 0xF058 or 0xF057),
      text = tostring(code),
      tone = succeeded and 'ok' or 'bad',
    }
  end,
}
