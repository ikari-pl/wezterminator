-- The active pane's working directory. Lua-native: needs no stats cache.

local common = require 'wzt.segments.common'

-- get_current_working_dir() returns a Url object in WezTerm 20240203 and a
-- plain string in some builds and tests.
local function path_of(cwd)
  if cwd == nil then
    return nil
  end
  if type(cwd) == 'string' then
    return (cwd:gsub('^file://[^/]*', ''))
  end
  local ok, p = pcall(function()
    return cwd.file_path
  end)
  return ok and p or nil
end

return {
  id = 'cwd',
  side = 'right',
  render = function(ctx)
    local pane = ctx.pane
    if not pane then
      return nil
    end
    local ok, cwd = pcall(function()
      return pane:get_current_working_dir()
    end)
    local path = ok and path_of(cwd) or nil
    local short = common.short_path(path, ctx.home)
    if not short then
      return nil
    end
    return { icon = common.glyph(0xF07B), text = short, tone = 'info' }
  end,
}
