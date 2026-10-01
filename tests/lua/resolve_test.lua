-- Runs every tests/fixtures/resolution/resolve-*.json through plugin/wzt/resolve.lua.
--
-- Each fixture runs twice, because the engine sees JSON in two shapes:
--
--   tagged     arrays carry the __wzt_array marker, nulls are dropped. This is
--              the clean view, what the Rust twin effectively sees.
--   wezterm    decoded like wezterm.json_parse: nulls vanish AND empty arrays
--              are plain empty tables. resolve.lua must restore the
--              array/object distinction from the schema alone.
--
-- Both must produce the expected output exactly (data-model.md, "Fixtures").

local T = ...
local resolve = T.resolve
local json = T.json

local dir = T.root .. '/tests/fixtures/resolution'
local fixtures = T.glob(dir .. '/resolve-*.json')

T.test('fixture set is present (a broken glob must not pass silently)', function()
  T.ok(#fixtures >= 15, 'expected at least 15 resolve-*.json fixtures, found ' .. #fixtures)
end)

local function run_case(case, input)
  return resolve.resolve(input)
end

local function check(case, output)
  -- 1. `expected`: top-level keys, exact deep equality.
  for key, want in pairs(case.expected or {}) do
    local diff = T.match(want, output[key], '/' .. key)
    if diff then
      T.fail(case.name .. ' expected.' .. key .. ' -> ' .. diff)
    end
  end
  -- 2. `expect_at`: JSON Pointers.
  local ptrs = {}
  for ptr in pairs(case.expect_at or {}) do
    ptrs[#ptrs + 1] = ptr
  end
  table.sort(ptrs)
  for _, ptr in ipairs(ptrs) do
    local want = case.expect_at[ptr]
    local found, got = T.pointer(output, ptr)
    if want == json.NULL then
      if found then
        T.fail(case.name .. ' expect_at ' .. ptr .. ': expected null/absent, got ' .. T.show(got))
      end
    else
      if not found then
        T.fail(case.name .. ' expect_at ' .. ptr .. ': path does not exist')
      end
      local diff = T.match(want, got, ptr)
      if diff then
        T.fail(case.name .. ' expect_at -> ' .. diff)
      end
    end
  end
  -- 3. `expect_absent`.
  for _, ptr in ipairs(case.expect_absent or {}) do
    local found, got = T.pointer(output, ptr)
    if found then
      T.fail(case.name .. ' expect_absent ' .. ptr .. ': exists with ' .. T.show(got))
    end
  end
  -- 4. No comment key may survive resolution.
  local hit, key = T.has_comment_key(output)
  if hit then
    T.fail(case.name .. ': output contains comment key ' .. tostring(key))
  end
end

for _, path in ipairs(fixtures) do
  local file = path:match('([^/]+)%.json$')
  local text = T.read_file(path)

  T.test(file .. ' [tagged]', function()
    local case = json.decode(text, { mark_arrays = true, null = json.NULL })
    T.eq(case.name, file, 'fixture name must equal its file name')
    T.eq(case.kind, 'resolution')
    -- Inputs: nulls already removed, per the resolution contract.
    local input = json.decode(text, { mark_arrays = true }).input
    check(case, run_case(case, input))
  end)

  T.test(file .. ' [wezterm json_parse]', function()
    local case = json.decode(text, { mark_arrays = true, null = json.NULL })
    local lossy = T.stub.new().wezterm.json_parse(text)
    check(case, run_case(case, lossy.input))
  end)
end

---------------------------------------------------------------------------
-- Unit tests for the pure helpers
---------------------------------------------------------------------------

T.test('strip_comments removes _ keys at every depth and leaves the input alone', function()
  local doc = { _ = 'top', a = { _note = 'x', b = 1, c = { { _ = 'in array', d = 2 } } }, _a = 'about a' }
  local out = resolve.strip_comments(doc)
  T.deep_eq(out, { a = { b = 1, c = { { d = 2 } } } })
  T.eq(doc._, 'top', 'input must not be mutated')
  T.eq(doc.a._note, 'x')
end)

T.test('strip_comments restores the empty-array marker from the schema', function()
  local out = resolve.strip_comments({ parts = { status = { segments = {} }, font = { corrections = {} } } })
  T.ok(resolve.is_array(out.parts.status.segments), 'segments is always an array')
  T.ok(not resolve.is_array(out.parts.font.corrections), 'corrections is an object')
end)

T.test('strip_comments does not treat user-chosen map keys as schema keys', function()
  -- A font family literally named "preferred" or a layer id "segments" must not
  -- turn an empty map value into an array.
  local out = resolve.strip_comments({ parts = { font = { corrections = { preferred = {} } } } })
  T.ok(not resolve.is_array(out.parts.font.corrections.preferred))
end)

T.test('merge: empty array replaces, empty object merges', function()
  local base = { list = { 1, 2 }, obj = { a = 1 } }
  local out = resolve.merge(base, { list = resolve.new_array(), obj = {} })
  T.ok(resolve.is_array(out.list) and #out.list == 0, 'empty array clears the list')
  T.deep_eq(out.obj, { a = 1 })
end)

T.test('merge: an object over a non-object creates the object', function()
  local out = resolve.merge({ x = 1 }, { x = { y = 2 } })
  T.deep_eq(out, { x = { y = 2 } })
end)

T.test('merge does not alias its inputs', function()
  local base = { a = { b = 1 } }
  local out = resolve.merge(base, { a = { c = 2 } })
  out.a.b = 99
  T.eq(base.a.b, 1)
end)

T.test('drop_owned: first owned key in table order is the one reported', function()
  local parts = { chrome = { tab_bar = { hidden = true } }, scheme = {}, palette = {}, font = {} }
  local over = resolve.drop_owned(parts, { 'colors', 'color_scheme' })
  -- color_scheme precedes colors in the table, so it reports `scheme`;
  -- colors reports what is left (`palette`) and `scheme` is not repeated.
  local by_path = {}
  for _, o in ipairs(over) do
    by_path[o.path] = o.config_key
  end
  T.eq(by_path.scheme, 'color_scheme')
  T.eq(by_path.palette, 'colors')
  T.eq(#over, 2)
  T.ok(over[1].path < over[2].path, 'sorted by path')
end)

T.test('drop_owned: a path that is not present is not reported', function()
  local over = resolve.drop_owned({ chrome = {} }, { 'window_background_opacity' })
  T.eq(#over, 0)
end)

T.test('resolve never reads an ignored document', function()
  local out = resolve.resolve({
    engine = { supported_schema_version = 1, default_preset = 'builtin:x' },
    layers = { builtin = { presets = { { schema_version = 2, id = 'builtin:x', name = 'X', parts = {} } } } },
  })
  T.eq(out.ignored[1].reason, 'unsupported_schema_version')
  T.eq(out.ignored[1].found, 2)
  T.eq(out.error.code, 'no_resolvable_preset')
  T.eq(out.resolved, nil)
end)
