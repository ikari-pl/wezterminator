-- The window's workspace, in the LEFT status area. Without it nothing says
-- workspaces exist, which makes them easy to own and never use. The icon
-- changes colour while the leader key is armed. Lua-native.

local common = require 'wzt.segments.common'

return {
  id = 'workspace',
  side = 'left',
  render = function(ctx)
    local window = ctx.window
    if not window then
      return nil
    end
    local ok, name = pcall(function()
      return window:active_workspace()
    end)
    if not ok or type(name) ~= 'string' or name == '' then
      return nil
    end
    local armed = false
    pcall(function()
      armed = window:leader_is_active() and true or false
    end)
    return {
      -- oct-repo: workspaces map one-to-one onto project repos.
      icon = common.glyph(0xF401),
      text = name,
      tone = armed and 'accent_alt' or (name == 'default' and 'fg_dim' or 'accent'),
    }
  end,
}
