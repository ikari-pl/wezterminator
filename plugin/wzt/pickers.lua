-- Colour-scheme and font pickers as InputSelector menus with LIVE PREVIEW.
-- They replace metis's fzf-in-a-split-pane scripts (zsh, fzf and OSC user
-- vars), so there is nothing to install and they work on Windows.
--
-- PREVIEW goes through the aggregator's `preview` channel with owner
-- `picker`: no expiry (the TUI's previews expire so a crashed TUI cannot leave
-- a window stuck; a Lua picker cannot crash independently of the GUI), and the
-- picker clears it itself when its menu closes.
--
-- WHAT InputSelector CAN DO (WezTerm 20240203): its callback fires once, when
-- the menu closes with a pick or a cancel. It cannot report the highlighted
-- row, so the preview happens per PICK, in a loop:
--
--   1. list        Enter on a row previews it in this window
--   2. confirm     "Keep X" / "Choose another" / Esc to revert
--
-- The preview stays while the confirm menu is open and is cleared when that
-- menu closes by any route other than a successful commit. A successful
-- commit leaves it for the reload to retire: the aggregator drops a preview
-- when it sees a new generation, so the window never flickers back to the old
-- look in between.
--
-- COMMIT writes the user's local overrides.json (`parts.scheme` or
-- `parts.font.preferred`), the layer that follows the user across presets.
-- The write happens from this action handler, never during config evaluation,
-- and the watched file triggers the reload.

local wezterm = require 'wezterm'
local apply = require 'wzt.apply'
local data = require 'wzt.data'
local overrides = require 'wzt.overrides'
local platform = require 'wzt.platform'
local resolve = require 'wzt.resolve'

local M = {}

local function copy(v)
  if type(v) ~= 'table' then
    return v
  end
  local out = {}
  for k, x in pairs(v) do
    out[k] = copy(x)
  end
  return out
end

local function toast(window, text)
  pcall(function()
    window:toast_notification('wezterminator', text, nil, 3000)
  end)
end

---------------------------------------------------------------------------
-- Choices and payloads
---------------------------------------------------------------------------

--- Built-in colour scheme names, sorted.
function M.scheme_names()
  local fn = wezterm.color and wezterm.color.get_builtin_schemes or wezterm.get_builtin_color_schemes
  local ok, schemes = pcall(fn)
  local names = {}
  if ok and type(schemes) == 'table' then
    for name in pairs(schemes) do
      names[#names + 1] = name
    end
  end
  table.sort(names)
  return names
end

--- Font families offered: the preset's preferred and fallback lists, then
--- `extra` (an `opts.fonts` list from the user's config). System font
--- enumeration belongs to the font unit.
function M.font_names(resolved, extra)
  local names, seen = {}, {}
  local function add(n)
    if type(n) == 'string' and n ~= '' and not seen[n] then
      seen[n] = true
      names[#names + 1] = n
    end
  end
  local font = resolved and resolved.parts.font
  for _, n in ipairs(font and font.preferred or {}) do
    add(n)
  end
  for _, n in ipairs(font and font.fallback or {}) do
    add(n)
  end
  for _, n in ipairs(extra or {}) do
    add(n)
  end
  return names
end

--- The preview payload for trying scheme `name`: the config keys that change,
--- built by the same function as the base config so the two cannot drift.
--- Overrides replace whole keys, so `colors` is sent too: without it the base
--- config's terminal colours would still sit on top of the previewed scheme.
function M.scheme_payload(resolved, host, name)
  local parts = copy(resolved.parts)
  parts.scheme = { wezterm_scheme = name }
  local fragment = apply.fragment({ parts = parts }, host)
  return { config = { color_scheme = fragment.color_scheme, colors = fragment.colors } }
end

--- The preview payload for trying font family `name` first, keeping the
--- preset's fallback list and size rules (including per-font corrections).
function M.font_payload(resolved, host, name)
  local parts = copy(resolved.parts)
  local effective = { name }
  local font = parts.font or {}
  for _, f in ipairs(font.fallback or {}) do
    if f ~= name then
      effective[#effective + 1] = f
    end
  end
  font.effective = effective
  parts.font = font
  local fragment = apply.fragment({ parts = parts }, host)
  return { config = { font = fragment.font, font_size = fragment.font_size } }
end

---------------------------------------------------------------------------
-- Commit (overrides.json)
---------------------------------------------------------------------------

--- Apply `mutate(parts)` to the local overrides document and write it
--- atomically. Comment keys and unrelated parts survive. Returns true, or
--- false and a reason. Refuses to overwrite a file it cannot read.
function M.write_override(dirs, mutate)
  local path = dirs['local'] .. '/overrides.json'
  local text = platform.read_file(path)
  local doc = {}
  if text then
    local decoded, err = data.decode(text)
    if type(decoded) ~= 'table' then
      return false, 'overrides.json is unreadable: ' .. tostring(err or 'not an object')
    end
    doc = resolve.normalize(decoded)
  end
  doc.schema_version = doc.schema_version or 1
  doc.parts = type(doc.parts) == 'table' and doc.parts or {}
  mutate(doc.parts)
  return platform.write_atomic(path, data.encode(doc))
end

function M.commit_scheme(dirs, name)
  return M.write_override(dirs, function(parts)
    parts.scheme = { wezterm_scheme = name }
  end)
end

function M.commit_font(dirs, name)
  return M.write_override(dirs, function(parts)
    local font = type(parts.font) == 'table' and parts.font or {}
    font.preferred = resolve.new_array({ name })
    parts.font = font
  end)
end

---------------------------------------------------------------------------
-- The loop
---------------------------------------------------------------------------

-- kind specs: how each picker lists, previews, commits and labels.
local KINDS = {
  scheme = {
    part = 'scheme',
    title = 'Colour scheme  (Enter previews)',
    names = function(ctx)
      return M.scheme_names()
    end,
    current = function(ctx)
      local s = ctx.resolved.parts.scheme
      return s and s.wezterm_scheme
    end,
    payload = M.scheme_payload,
    commit = M.commit_scheme,
    noun = 'Scheme',
  },
  font = {
    part = 'font',
    title = 'Font  (Enter previews)',
    names = function(ctx)
      return M.font_names(ctx.resolved, ctx.fonts)
    end,
    current = function(ctx)
      local f = ctx.resolved.parts.font
      return f and f.effective and f.effective[1]
    end,
    payload = M.font_payload,
    commit = M.commit_font,
    noun = 'Font',
  },
}

local open

local function confirm(window, pane, kind, ctx, name)
  local spec = KINDS[kind]
  local act = wezterm.action
  window:perform_action(
    act.InputSelector({
      title = spec.noun .. ': ' .. name,
      description = 'Previewing in this window. Esc reverts.',
      choices = {
        { id = 'keep', label = 'Keep ' .. name },
        { id = 'another', label = 'Choose another…' },
        { id = 'revert', label = 'Revert' },
      },
      fuzzy = false,
      action = wezterm.action_callback(function(win, p, id)
        if id == 'keep' then
          local ok, err = spec.commit(ctx.dirs, name)
          if ok then
            toast(win, spec.noun .. ': ' .. name .. ' (applies to every preset)')
          else
            overrides.preview_clear(win)
            toast(win, 'Cannot save ' .. spec.noun:lower() .. ': ' .. tostring(err))
          end
        elseif id == 'another' then
          open(win, p, kind, ctx)
        else
          overrides.preview_clear(win)
        end
      end),
    }),
    pane)
end

--- Open a picker. kind: 'scheme' or 'font'. ctx = {
---   resolved, host, dirs,
---   fonts = optional extra family names
--- }
--- Returns true when the menu was shown.
function open(window, pane, kind, ctx)
  local spec = KINDS[kind]
  if not ctx.resolved or not ctx.resolved.parts[spec.part] then
    -- Add-on mode with the user's own colour_scheme / font set: the part was
    -- overruled, and previewing or saving it would change nothing.
    overrides.preview_clear(window)
    toast(window, 'Your config sets the ' .. spec.noun:lower() .. '; the picker is off')
    return false
  end
  local names = spec.names(ctx)
  if #names == 0 then
    toast(window, 'No ' .. spec.noun:lower() .. 's to choose from')
    return false
  end

  local current = spec.current(ctx)
  local choices = {}
  for _, n in ipairs(names) do
    choices[#choices + 1] = { id = n, label = (n == current and '● ' or '○ ') .. n }
  end

  window:perform_action(
    wezterm.action.InputSelector({
      title = spec.title,
      choices = choices,
      fuzzy = true,
      action = wezterm.action_callback(function(win, p, id)
        if not id then
          overrides.preview_clear(win)
          return
        end
        local payload = spec.payload(ctx.resolved, ctx.host or {}, id)
        overrides.preview_set(win, payload, { owner = 'picker' })
        confirm(win, p, kind, ctx, id)
      end),
    }),
    pane)
  return true
end
M.open = open

--- Callback actions by name, for keys.lua and the palette.
function M.actions(ctx)
  return {
    pick_scheme = wezterm.action_callback(function(window, pane)
      open(window, pane, 'scheme', ctx)
    end),
    pick_font = wezterm.action_callback(function(window, pane)
      open(window, pane, 'font', ctx)
    end),
  }
end

return M
