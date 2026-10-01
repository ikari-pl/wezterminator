-- Rich menus behind the status bar segments.
--
-- The status bar cannot be clicked: WezTerm's mouse_bindings fire without
-- telling Lua WHERE the pointer was, so a segment cannot be a click target.
-- These menus are the next best thing -- the same information, one key away,
-- with actions attached (keys.lua binds them).
--
-- Everything here runs ON DEMAND only. `ps` costs tens of milliseconds, which
-- is fine for a keypress and ruinous in the once-a-second status loop.

local wezterm = require 'wezterm'
local platform = require 'wzt.platform'
local common = require 'wzt.segments.common'

local M = {}

--- Rows shown per process menu.
M.ROWS = 18

local function trim(s)
  return (s:gsub('^%s+', ''):gsub('%s+$', ''))
end

local function toast(window, text, ms)
  pcall(function()
    window:toast_notification('wezterminator', text, nil, ms or 3000)
  end)
end

---------------------------------------------------------------------------
-- Processes
---------------------------------------------------------------------------

--- The `ps` command line that lists processes sorted by `mode` ('cpu' or
--- 'mem'), or nil where there is no ps (Windows). ps does the sorting so Lua
--- never sorts.
function M.ps_command(os_name, mode)
  if os_name == 'macos' then
    return { 'ps', '-Aceo', 'pid,%cpu,rss,comm', mode == 'cpu' and '-r' or '-m' }
  elseif os_name == 'linux' then
    return { 'ps', '-Ao', 'pid,pcpu,rss,comm', mode == 'cpu' and '--sort=-pcpu' or '--sort=-rss' }
  end
  return nil
end

--- Parse ps output into { {pid =, cpu =, gb =, comm =}, ... }, at most `rows`.
function M.parse_ps(stdout, rows)
  local out, first = {}, true
  for line in (stdout or ''):gmatch('[^\n]+') do
    if first then
      first = false -- header row
    elseif #out < (rows or M.ROWS) then
      local pid, cpu, rss, comm = line:match('^%s*(%d+)%s+([%d.]+)%s+(%d+)%s+(.+)$')
      if pid then
        out[#out + 1] = {
          pid = pid,
          cpu = tonumber(cpu) or 0,
          gb = (tonumber(rss) or 0) / 1048576, -- ps reports rss in KiB
          comm = trim(comm),
        }
      end
    end
  end
  return out
end

--- Second-stage menu for one process. Kill lives here rather than on the first
--- selection, so terminating something is always a deliberate second choice
--- and never a stray Enter on a fuzzy list.
local function process_actions(window, pane, proc)
  local act = wezterm.action
  local choices = {
    { id = 'htop', label = common.glyph(0xF0238) .. '  Watch in htop' },
    { id = 'copy', label = common.glyph(0xF014D) .. '  Copy PID ' .. proc.pid },
    { id = 'term', label = common.glyph(0xF073A) .. '  Terminate (SIGTERM) ' .. proc.comm .. ' [' .. proc.pid .. ']' },
    { id = 'kill', label = common.glyph(0xF068C) .. '  Force kill (SIGKILL) ' .. proc.comm .. ' [' .. proc.pid .. ']' },
  }
  window:perform_action(
    act.InputSelector({
      title = proc.comm .. '  [' .. proc.pid .. ']',
      choices = choices,
      fuzzy = false,
      action = wezterm.action_callback(function(win, p, id)
        if id == 'htop' then
          win:perform_action(act.SpawnCommandInNewTab({ args = { 'htop', '-p', proc.pid } }), p)
        elseif id == 'copy' then
          pcall(function()
            win:copy_to_clipboard(proc.pid)
          end)
          toast(win, 'Copied PID ' .. proc.pid, 2000)
        elseif id == 'term' or id == 'kill' then
          wezterm.background_child_process({ 'kill', id == 'term' and '-TERM' or '-KILL', proc.pid })
          toast(win, (id == 'term' and 'Terminated ' or 'Killed ') .. proc.comm, 2500)
        end
      end),
    }),
    pane)
end

--- Open the top-processes menu in `window`. Returns true when it was shown.
function M.show_processes(window, pane, mode, title, os_name)
  local cmd = M.ps_command(os_name or platform.os(), mode)
  if not cmd then
    toast(window, 'The process menu needs ps, which this platform does not have')
    return false
  end
  local ok, success, stdout = pcall(wezterm.run_child_process, cmd)
  local list = (ok and success) and M.parse_ps(stdout, M.ROWS) or {}
  if #list == 0 then
    toast(window, 'Could not read the process table')
    return false
  end

  local choices, by_id = {}, {}
  for i, p in ipairs(list) do
    by_id[p.pid] = p
    choices[#choices + 1] = {
      id = p.pid,
      label = string.format('%2d  %6.1f%%  %6.2f GB   %s', i, p.cpu, p.gb, p.comm),
    }
  end
  window:perform_action(
    wezterm.action.InputSelector({
      title = title,
      choices = choices,
      fuzzy = true,
      action = wezterm.action_callback(function(win, p, id)
        local proc = id and by_id[id]
        if proc then
          process_actions(win, p, proc)
        end
      end),
    }),
    pane)
  return true
end

---------------------------------------------------------------------------
-- Workspaces
---------------------------------------------------------------------------

--- Pane counts and a sample title per workspace, from the mux.
function M.workspace_info()
  local counts, titles = {}, {}
  for _, win in ipairs(wezterm.mux.all_windows()) do
    local ws = win:get_workspace()
    for _, tab in ipairs(win:tabs()) do
      for _, pi in ipairs(tab:panes_with_info()) do
        counts[ws] = (counts[ws] or 0) + 1
        if not titles[ws] then
          titles[ws] = pi.pane:get_title()
        end
      end
    end
  end
  return counts, titles
end

--- Richer than the plain switcher, because it shows what is actually alive in
--- each workspace. With a persistent mux a workspace can hold running
--- processes with no window on screen.
function M.show_workspaces(window, pane)
  local act = wezterm.action
  local counts, titles = M.workspace_info()
  local active = window:active_workspace()
  local choices = {}
  for _, ws in ipairs(wezterm.mux.get_workspace_names()) do
    local n = counts[ws] or 0
    choices[#choices + 1] = {
      id = ws,
      label = string.format('%s  %-28s %d pane%s   %s',
        ws == active and '●' or '○', ws, n, n == 1 and '' or 's', titles[ws] or ''),
    }
  end
  if #choices == 0 then
    toast(window, 'No workspaces', 2000)
    return false
  end

  window:perform_action(
    act.InputSelector({
      title = 'Workspaces  (' .. #choices .. ')',
      choices = choices,
      fuzzy = true,
      action = wezterm.action_callback(function(win, p, id)
        if not id then
          return
        end
        win:perform_action(
          act.InputSelector({
            title = 'Workspace: ' .. id,
            choices = {
              { id = 'switch', label = common.glyph(0xF01BE) .. '  Switch to it' },
              { id = 'rename', label = common.glyph(0xF0455) .. '  Rename it' },
            },
            fuzzy = false,
            action = wezterm.action_callback(function(w2, p2, choice)
              if choice == 'switch' then
                w2:perform_action(act.SwitchToWorkspace({ name = id }), p2)
              elseif choice == 'rename' then
                w2:perform_action(
                  act.PromptInputLine({
                    description = 'Rename workspace "' .. id .. '" to',
                    action = wezterm.action_callback(function(_, _, line)
                      if line and line ~= '' then
                        wezterm.mux.rename_workspace(id, line)
                      end
                    end),
                  }),
                  p2)
              end
            end),
          }),
          p)
      end),
    }),
    pane)
  return true
end

---------------------------------------------------------------------------
-- Actions
---------------------------------------------------------------------------

--- Callback actions by name, for keys.lua and the palette.
function M.actions()
  return {
    cpu_menu = wezterm.action_callback(function(window, pane)
      M.show_processes(window, pane, 'cpu', 'Top processes by CPU')
    end),
    memory_menu = wezterm.action_callback(function(window, pane)
      M.show_processes(window, pane, 'mem', 'Top processes by memory')
    end),
    workspace_menu = wezterm.action_callback(function(window, pane)
      M.show_workspaces(window, pane)
    end),
  }
end

return M
