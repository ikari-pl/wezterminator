-- Test runner for the Lua engine.
--
--   lua tests/lua/run.lua            # stock Lua (5.1 through 5.5) or luajit
--   lua tests/lua/run.lua resolve    # only test files whose name contains "resolve"
--
-- It discovers tests/lua/*_test.lua. Each file is a chunk that receives the
-- harness table `T` and registers tests with T.test(name, fn). Exits non-zero
-- if any test fails, so CI can use it directly.

local script = (arg and arg[0]) or 'tests/lua/run.lua'
local here = script:match('^(.*)/[^/]*$') or '.'
local root = here .. '/../..'

-- Make the engine and the helpers loadable.
package.path = table.concat({
  here .. '/?.lua',
  root .. '/plugin/?.lua',
  root .. '/plugin/?/init.lua',
  package.path,
}, ';')

local resolve = require 'wzt.resolve'
local json = require 'json'
local stub_lib = require 'wezterm_stub'

local T = {
  root = root,
  here = here,
  json = json,
  stub = stub_lib,
  resolve = resolve,
}

---------------------------------------------------------------------------
-- Registry
---------------------------------------------------------------------------

local tests = {}
local current_file = '?'

function T.test(name, fn)
  tests[#tests + 1] = { file = current_file, name = name, fn = fn }
end

---------------------------------------------------------------------------
-- Assertions
---------------------------------------------------------------------------

local function show(v, depth)
  depth = depth or 0
  if type(v) == 'string' then
    return string.format('%q', v)
  elseif type(v) ~= 'table' then
    return tostring(v)
  elseif v == json.NULL then
    return 'null'
  elseif depth > 4 then
    return '{...}'
  end
  local keys = {}
  for k in pairs(v) do
    keys[#keys + 1] = k
  end
  table.sort(keys, function(a, b)
    return tostring(a) < tostring(b)
  end)
  local parts = {}
  for _, k in ipairs(keys) do
    parts[#parts + 1] = tostring(k) .. '=' .. show(v[k], depth + 1)
  end
  local open, close = '{', '}'
  if resolve.is_array(v) then
    open, close = '[', ']'
  end
  return open .. table.concat(parts, ', ') .. close
end
T.show = show

function T.fail(msg)
  error({ test_failure = msg }, 0)
end

function T.ok(cond, msg)
  if not cond then
    T.fail(msg or 'expected a truthy value')
  end
end

function T.eq(actual, expected, msg)
  if actual ~= expected then
    T.fail(string.format('%sexpected %s, got %s', msg and (msg .. ': ') or '', show(expected), show(actual)))
  end
end

function T.deep_eq(actual, expected, msg)
  if not resolve.deep_equal(actual, expected) then
    T.fail(string.format('%sexpected %s\n        got      %s', msg and (msg .. ': ') or '', show(expected), show(actual)))
  end
end

--- Run fn and require it to raise an error containing `pattern` (plain find).
function T.raises(fn, pattern, msg)
  local ok, err = pcall(fn)
  if ok then
    T.fail((msg or 'expected an error') .. ' but the call succeeded')
  end
  if pattern and not tostring(type(err) == 'table' and (err.test_failure or '') or err):find(pattern, 1, true) then
    T.fail(string.format('error %s does not contain %q', tostring(err), pattern))
  end
end

--- Expected-vs-actual matcher for fixtures. `expected` was decoded with the
--- NULL sentinel and array markers; `actual` is the engine's output.
--- Returns nil on match or a string describing the first difference.
function T.match(expected, actual, path)
  path = path or ''
  if expected == json.NULL then
    if actual ~= nil then
      return string.format('%s: expected null, got %s', path ~= '' and path or '/', show(actual))
    end
    return nil
  end
  if type(expected) ~= 'table' then
    if expected ~= actual then
      return string.format('%s: expected %s, got %s', path ~= '' and path or '/', show(expected), show(actual))
    end
    return nil
  end
  if type(actual) ~= 'table' then
    return string.format('%s: expected %s, got %s', path ~= '' and path or '/', show(expected), show(actual))
  end
  local e_arr, a_arr = resolve.is_array(expected), resolve.is_array(actual)
  if e_arr ~= a_arr then
    return string.format('%s: expected %s, got %s', path ~= '' and path or '/', e_arr and 'an array' or 'an object', a_arr and 'an array' or 'an object')
  end
  if e_arr then
    if #expected ~= #actual then
      return string.format('%s: expected %d items, got %d (%s)', path ~= '' and path or '/', #expected, #actual, show(actual))
    end
    for i = 1, #expected do
      local d = T.match(expected[i], actual[i], path .. '/' .. (i - 1))
      if d then
        return d
      end
    end
    return nil
  end
  for k, v in pairs(expected) do
    local d = T.match(v, actual[k], path .. '/' .. tostring(k))
    if d then
      return d
    end
  end
  for k, v in pairs(actual) do
    if expected[k] == nil then
      return string.format('%s/%s: unexpected key (value %s)', path, tostring(k), show(v))
    end
  end
  return nil
end

--- Resolve an RFC 6901 JSON Pointer against a decoded document. Returns
--- found, value.
function T.pointer(doc, ptr)
  if ptr == '' then
    return true, doc
  end
  local node = doc
  for raw in ptr:gmatch('/([^/]*)') do
    local seg = raw:gsub('~1', '/'):gsub('~0', '~')
    if type(node) ~= 'table' then
      return false
    end
    if resolve.is_array(node) then
      local i = tonumber(seg)
      if not i or node[i + 1] == nil then
        return false
      end
      node = node[i + 1]
    else
      if node[seg] == nil then
        return false
      end
      node = node[seg]
    end
  end
  return true, node
end

--- True if any object key anywhere in `v` starts with an underscore.
function T.has_comment_key(v)
  if type(v) ~= 'table' then
    return false
  end
  for k, x in pairs(v) do
    if type(k) == 'string' and k:sub(1, 1) == '_' then
      return true, k
    end
    local hit, key = T.has_comment_key(x)
    if hit then
      return true, key
    end
  end
  return false
end

function T.read_file(path)
  local f = assert(io.open(path, 'r'))
  local text = f:read('*a')
  f:close()
  return text
end

--- Files matching a shell glob, sorted.
function T.glob(pattern)
  local p = io.popen('ls -1 ' .. pattern .. ' 2>/dev/null')
  local out = {}
  for line in p:lines() do
    out[#out + 1] = line
  end
  p:close()
  table.sort(out)
  return out
end

---------------------------------------------------------------------------
-- Discovery and execution
---------------------------------------------------------------------------

local filter = arg and arg[1]
local files = T.glob(here .. '/*_test.lua')
for _, path in ipairs(files) do
  local name = path:match('([^/]+)$')
  if not filter or name:find(filter, 1, true) then
    current_file = name
    local chunk, err = loadfile(path)
    if not chunk then
      io.stderr:write('cannot load ' .. path .. ': ' .. tostring(err) .. '\n')
      os.exit(2)
    end
    -- Test files either take T as the chunk argument (`function(T) ... end`
    -- as the whole file body via `local T = ...`) or `return function(T)`.
    local ok, ret = pcall(chunk, T)
    if not ok then
      io.stderr:write('error loading ' .. path .. ': ' .. tostring(ret) .. '\n')
      os.exit(2)
    end
    if type(ret) == 'function' then
      ret(T)
    end
  end
end

local passed, failed = 0, {}
local last_file
for _, t in ipairs(tests) do
  if t.file ~= last_file then
    print('\n' .. t.file)
    last_file = t.file
  end
  local ok, err = xpcall(t.fn, function(e)
    if type(e) == 'table' and e.test_failure then
      return e
    end
    return { crash = tostring(e), trace = debug.traceback('', 2) }
  end)
  if ok then
    passed = passed + 1
    print('  ok    ' .. t.name)
  else
    failed[#failed + 1] = { test = t, err = err }
    print('  FAIL  ' .. t.name)
    local detail = err.test_failure or ('error: ' .. tostring(err.crash) .. (err.trace or ''))
    for line in tostring(detail):gmatch('[^\n]+') do
      print('          ' .. line)
    end
  end
end

print(string.format('\n%d passed, %d failed, %d total  (%s)', passed, #failed, #tests, _VERSION .. (jit and (' / ' .. jit.version) or '')))
if #failed > 0 then
  os.exit(1)
end
if #tests == 0 then
  print('no tests found')
  os.exit(2)
end
