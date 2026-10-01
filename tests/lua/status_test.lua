-- Status / sparkline / segment visibility / keys / quick-select (U3).

local T = ...
local stub_lib = T.stub
local common = require 'wzt.segments.common'
local keys = require 'wzt.keys'

local function utf8_len(s)
  local n = 0
  for _ in s:gmatch('[%z\1-\127\194-\244][\128-\191]*') do
    n = n + 1
  end
  return n
end

--- Modules that `require 'wezterm'` at load time.
local function with_wezterm(fn)
  local stub = stub_lib.new()
  package.loaded['wezterm'] = stub.wezterm
  package.loaded['wzt.quickselect'] = nil
  package.loaded['wzt.segments.warp'] = nil
  package.loaded['wzt.segments.tailscale'] = nil
  package.loaded['wzt.segments.load'] = nil
  package.loaded['wzt.segments.memory'] = nil
  local ok, err = pcall(fn, stub)
  stub.cleanup()
  package.loaded['wezterm'] = nil
  package.loaded['wzt.quickselect'] = nil
  package.loaded['wzt.segments.warp'] = nil
  package.loaded['wzt.segments.tailscale'] = nil
  package.loaded['wzt.segments.load'] = nil
  package.loaded['wzt.segments.memory'] = nil
  if not ok then
    error(err, 0)
  end
end

T.test('sparkline: flat series renders mid-height glyphs at fixed width', function()
  local buf = {}
  for _ = 1, common.SAMPLES do
    common.push(buf, 0.5, common.SAMPLES)
  end
  local spark = common.sparkline(buf, common.SAMPLES)
  T.eq(utf8_len(spark), common.SAMPLES)
  local mid = common.BLOCKS[math.floor(0.5 * (#common.BLOCKS - 1) + 0.5) + 1]
  local count = 0
  for _ in spark:gmatch(mid) do
    count = count + 1
  end
  T.ok(count >= common.SAMPLES - 1, 'flat mid series uses mid blocks, got ' .. tostring(count))
end)

T.test('sparkline: partially filled buffer pads to fixed width', function()
  local buf = {}
  common.push(buf, 1.0, common.SAMPLES)
  common.push(buf, 0.0, common.SAMPLES)
  T.eq(utf8_len(common.sparkline(buf, common.SAMPLES)), common.SAMPLES)
end)

T.test('AE7: missing warp key hides WARP; ts=up renders Tailscale', function()
  with_wezterm(function()
    local warp = require 'wzt.segments.warp'
    local ts = require 'wzt.segments.tailscale'
    T.eq(warp.render({ stats = { ts = 'up' } }), nil, 'warp hidden without key')
    T.ok(ts.render({ stats = { ts = 'up' } }) ~= nil, 'tailscale shows for ts=up')
    T.ok(warp.render({ stats = { warp = 'up' } }) ~= nil, 'warp shows when present')
  end)
end)

T.test('with no stats cache, stats-backed segments hide', function()
  with_wezterm(function()
    local load = require 'wzt.segments.load'
    local memory = require 'wzt.segments.memory'
    T.eq(load.render({ stats = nil }), nil)
    T.eq(memory.render({ stats = nil }), nil)
  end)
end)

T.test('keys: identical user leader is a conflict; disjoint keys report none', function()
  local dummy = {}
  for _, spec in ipairs(keys.KEYS) do
    dummy[spec.action] = true
  end

  local clash = keys.plan({
    os = 'macos',
    mode = 'addon',
    actions = dummy,
    user_keys = {},
    user_leader = { key = 'Space', mods = 'CTRL|SHIFT' },
  })
  local leader_conflicts = 0
  for _, c in ipairs(clash.conflicts) do
    if c.kind == 'leader' then
      leader_conflicts = leader_conflicts + 1
    end
  end
  T.ok(leader_conflicts >= 1, 'identical leader is reported')

  local ok_plan = keys.plan({
    os = 'macos',
    mode = 'addon',
    actions = dummy,
    user_keys = { { key = 'F13', mods = 'CTRL' } },
    user_leader = { key = 'a', mods = 'CTRL' },
  })
  local key_conflicts = 0
  for _, c in ipairs(ok_plan.conflicts) do
    if c.kind == 'key' then
      key_conflicts = key_conflicts + 1
    end
  end
  T.eq(key_conflicts, 0, 'disjoint key set has no key conflicts')
end)

T.test('quick-select: issue key uses URL pattern; without pattern only copies', function()
  with_wezterm(function()
    local qs = require 'wzt.quickselect'
    local with_pat = qs.resolve('PROJ-123', {
      issue_url_pattern = 'https://example.com/issues/{key}',
    })
    T.eq(with_pat.kind, 'issue')
    T.eq(with_pat.open, 'https://example.com/issues/PROJ-123')

    local no_pat = qs.resolve('PROJ-123', {})
    T.eq(no_pat.kind, 'issue')
    T.eq(no_pat.copy, 'PROJ-123')
    T.eq(no_pat.open, nil)
  end)
end)
