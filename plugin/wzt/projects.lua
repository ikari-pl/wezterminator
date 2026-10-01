-- Project workspace picker.
--
-- Scans the machine's `project_roots` (one and two levels deep) for git
-- repositories and offers them in a fuzzy selector. Choosing one switches to
-- a WezTerm WORKSPACE named after the project, spawning a shell already in
-- its directory, so each project keeps its own independent tabs and panes.
--
-- There are NO personal defaults: with no `project_roots` in the machine
-- settings the picker says how to configure it instead of guessing a directory.

local wezterm = require 'wezterm'
local common = require 'wzt.segments.common'

local M = {}

--- Expand a leading `~/` against `home`. Other paths pass through.
function M.expand(root, home)
  if type(root) ~= 'string' or root == '' then
    return nil
  end
  if root == '~' then
    return home
  end
  if root:sub(1, 2) == '~/' then
    return (home or '') .. root:sub(2)
  end
  return root
end

--- The configured roots, expanded. Empty when none are set.
function M.roots(machine, home)
  local out = {}
  for _, r in ipairs(machine and machine.project_roots or {}) do
    local p = M.expand(r, home or wezterm.home_dir)
    if p then
      out[#out + 1] = (p:gsub('/+$', ''))
    end
  end
  return out
end

--- Find repositories under `roots`. `glob` defaults to wezterm.glob and is a
--- parameter so tests can supply a file tree. Returns
--- { {path =, workspace =}, ... } sorted by workspace name.
function M.discover(roots, glob)
  glob = glob or wezterm.glob
  local projects, seen = {}, {}
  for _, root in ipairs(roots) do
    -- Two patterns: repos directly under the root, and repos one level deeper
    -- (umbrella directories such as ai/ or tools/).
    for _, pattern in ipairs({ root .. '/*/.git', root .. '/*/*/.git' }) do
      local ok, markers = pcall(glob, pattern)
      for _, marker in ipairs(ok and markers or {}) do
        local path = marker:gsub('/%.git$', '')
        if not seen[path] then
          seen[path] = true
          local workspace = path
          if path:sub(1, #root + 1) == root .. '/' then
            workspace = path:sub(#root + 2)
          end
          projects[#projects + 1] = { path = path, workspace = workspace }
        end
      end
    end
  end
  table.sort(projects, function(a, b)
    return a.workspace < b.workspace
  end)
  return projects
end

local function toast(window, text)
  pcall(function()
    window:toast_notification('wezterminator', text, nil, 3500)
  end)
end

--- The InputSelector choices for a project list.
function M.choices(projects)
  local choices = {}
  for _, p in ipairs(projects) do
    choices[#choices + 1] = { id = p.path, label = common.glyph(0xF024B) .. '  ' .. p.workspace }
  end
  return choices
end

--- Open the picker in `window`.
function M.pick(window, pane, machine, opts)
  opts = opts or {}
  local roots = M.roots(machine, opts.home)
  if #roots == 0 then
    toast(window, 'No project_roots in machine.json; add the directories to scan')
    return false
  end
  local projects = M.discover(roots, opts.glob)
  if #projects == 0 then
    toast(window, 'No git projects found under ' .. table.concat(roots, ', '))
    return false
  end

  local act = wezterm.action
  local by_path = {}
  for _, p in ipairs(projects) do
    by_path[p.path] = p
  end
  window:perform_action(
    act.InputSelector({
      title = 'Open project workspace',
      choices = M.choices(projects),
      fuzzy = true,
      action = wezterm.action_callback(function(win, current_pane, id)
        local project = id and by_path[id] or nil
        if project then
          win:perform_action(
            act.SwitchToWorkspace({ name = project.workspace, spawn = { cwd = project.path } }),
            current_pane)
        end
      end),
    }),
    pane)
  return true
end

--- A callback action bound to the machine settings.
function M.action(machine, opts)
  return wezterm.action_callback(function(window, pane)
    M.pick(window, pane, machine, opts)
  end)
end

return M
