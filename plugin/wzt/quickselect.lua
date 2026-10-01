-- Quick-select: pick a URL, issue key or git SHA off the screen and open it.
-- Plus the small pane helpers that grew around it in metis (open the cwd,
-- open the GitHub repo, new tab here, edit a config file).
--
-- QuickSelectArgs highlights every match of the patterns and labels each with
-- a key; pressing that key selects it and fires the action. Which kind it was
-- is then decided from the selected TEXT. The selection is classified
-- structurally, not by re-running the user's regex (Lua patterns are not
-- regexes): a URL starts with http(s)://, a bare 7-40 character hex string is
-- a SHA, anything else the patterns offered is an issue key.
--
-- Machine settings, no personal defaults:
--   issue_key_pattern  regex recognising an issue key (generic default below)
--   issue_url_pattern  URL template with {key}. ABSENT means the key is only
--                      copied -- there is no built-in tracker
--   editor             { command, args } for the edit actions

local wezterm = require 'wezterm'

local M = {}

--- Generic "PROJ-123" shape. Not tied to any tracker or organisation.
M.DEFAULT_ISSUE_KEY = [[\b[A-Z][A-Z0-9]+-[0-9]+\b]]

local URL_PATTERN = [[https?://[^\s<>"']+]]
local SHA_PATTERN = [[\b[0-9a-fA-F]{7,40}\b]]

local function trim(s)
  return (s:gsub('^%s+', ''):gsub('%s+$', ''))
end

--- The regex list for QuickSelectArgs.
function M.patterns(machine)
  local key = machine and machine.issue_key_pattern or M.DEFAULT_ISSUE_KEY
  return { URL_PATTERN, SHA_PATTERN, key }
end

--- 'url', 'sha' or 'issue' for selected text.
function M.classify(text)
  if text:match('^https?://') then
    return 'url'
  end
  if text:match('^%x+$') and #text >= 7 and #text <= 40 then
    return 'sha'
  end
  return 'issue'
end

--- https://github.com/<owner>/<repo> from a git remote URL, or nil.
function M.github_url_from_remote(remote)
  remote = trim(remote or ''):gsub('%.git$', '')
  local repo = remote:match('^git@github%.com:(.+)$')
    or remote:match('^ssh://git@github%.com/(.+)$')
    or remote:match('^https?://github%.com/(.+)$')
  if repo then
    return 'https://github.com/' .. repo
  end
  return nil
end

--- Decide what to do with selected text. Pure.
---   github_url   function() -> repo URL or nil; called only for a SHA
--- Returns { kind =, open = <url> } or { kind =, copy = <text>, note = <why> }.
function M.resolve(text, machine, github_url)
  text = trim(text or '')
  if text == '' then
    return nil
  end
  local kind = M.classify(text)
  if kind == 'url' then
    return { kind = kind, open = text }
  elseif kind == 'issue' then
    local pattern = machine and machine.issue_url_pattern
    if pattern then
      return { kind = kind, open = (pattern:gsub('{key}', function()
        return text
      end)) }
    end
    return { kind = kind, copy = text, note = 'no issue_url_pattern in machine.json' }
  end
  local repo = github_url and github_url()
  if repo then
    return { kind = kind, open = repo .. '/commit/' .. text }
  end
  return { kind = kind, copy = text, note = 'no GitHub origin for this pane' }
end

---------------------------------------------------------------------------
-- Pane helpers
---------------------------------------------------------------------------

--- The pane's working directory as a path, or the home directory.
function M.pane_cwd(pane)
  local ok, cwd = pcall(function()
    return pane:get_current_working_dir()
  end)
  if ok and cwd then
    if type(cwd) == 'string' then
      return (cwd:gsub('^file://[^/]*', ''))
    end
    local okp, p = pcall(function()
      return cwd.file_path
    end)
    if okp and p then
      return p
    end
  end
  return wezterm.home_dir
end

function M.github_url_for_pane(pane)
  local ok, success, remote = pcall(wezterm.run_child_process, {
    'git', '-C', M.pane_cwd(pane), 'remote', 'get-url', 'origin',
  })
  if not ok or not success then
    return nil
  end
  return M.github_url_from_remote(remote)
end

local function toast(window, text)
  pcall(function()
    window:toast_notification('wezterminator', text, nil, 3000)
  end)
end

--- The command line that opens `path` in the configured editor, or nil and a
--- reason.
function M.editor_args(machine, path)
  local editor = machine and machine.editor
  if not editor or not editor.command then
    return nil, 'No editor in machine.json; set editor.command'
  end
  local args = { editor.command }
  for _, a in ipairs(editor.args or {}) do
    args[#args + 1] = a
  end
  args[#args + 1] = path
  return args
end

--- Callback actions by name. `ctx` = { machine =, edit = { overrides =, machine = } }.
function M.actions(ctx)
  local act = wezterm.action
  local machine = ctx.machine or {}
  local out = {}

  out.quick_select = act.QuickSelectArgs({
    label = 'Open URL, issue, or git commit',
    patterns = M.patterns(machine),
    action = wezterm.action_callback(function(window, pane)
      local selected = trim(window:get_selection_text_for_pane(pane) or '')
      local r = M.resolve(selected, machine, function()
        return M.github_url_for_pane(pane)
      end)
      if not r then
        return
      end
      if r.open then
        wezterm.open_with(r.open)
      else
        window:perform_action(act.CopyTo('Clipboard'), pane)
        toast(window, 'Copied ' .. r.copy .. (r.note and (' (' .. r.note .. ')') or ''))
      end
    end),
  })

  out.open_cwd = wezterm.action_callback(function(_, pane)
    wezterm.open_with(M.pane_cwd(pane))
  end)

  out.open_github = wezterm.action_callback(function(window, pane)
    local url = M.github_url_for_pane(pane)
    if url then
      wezterm.open_with(url)
    else
      toast(window, 'No GitHub origin for this pane')
    end
  end)

  out.tab_at_cwd = wezterm.action_callback(function(window, pane)
    window:perform_action(act.SpawnCommandInNewTab({ cwd = M.pane_cwd(pane) }), pane)
  end)

  local function editor_action(path)
    return wezterm.action_callback(function(window, pane)
      local args, why = M.editor_args(machine, path)
      if not args then
        toast(window, why)
        return
      end
      window:perform_action(act.SpawnCommandInNewTab({ args = args }), pane)
    end)
  end
  local edit = ctx.edit or {}
  if edit.overrides then
    out.edit_overrides = editor_action(edit.overrides)
  end
  if edit.machine then
    out.edit_machine = editor_action(edit.machine)
  end
  return out
end

return M
