-- Minimal JSON decoder for the test harness (stock Lua has none).
--
-- Two modes, because the fixtures need both views of the same bytes:
--
--   decode(text, { mark_arrays = true })   arrays carry the __wzt_array marker,
--                                          so an empty array differs from an
--                                          empty object. Used for EXPECTED values.
--   decode(text, { null = M.NULL })        keep nulls as a sentinel so "expected
--                                          null" can be told from "absent".
--
-- With no options the result mimics wezterm.json_parse: nulls vanish and empty
-- arrays are plain empty tables. That is the lossy representation the real
-- engine has to cope with.

local M = {}

M.NULL = setmetatable({}, { __tostring = function() return 'null' end })

local function utf8_char(cp)
  if utf8 and utf8.char then
    return utf8.char(cp)
  end
  if cp < 0x80 then
    return string.char(cp)
  elseif cp < 0x800 then
    return string.char(0xC0 + math.floor(cp / 0x40), 0x80 + cp % 0x40)
  elseif cp < 0x10000 then
    return string.char(0xE0 + math.floor(cp / 0x1000), 0x80 + math.floor(cp / 0x40) % 0x40, 0x80 + cp % 0x40)
  end
  return string.char(
    0xF0 + math.floor(cp / 0x40000),
    0x80 + math.floor(cp / 0x1000) % 0x40,
    0x80 + math.floor(cp / 0x40) % 0x40,
    0x80 + cp % 0x40
  )
end

local ESC = { ['"'] = '"', ['\\'] = '\\', ['/'] = '/', b = '\b', f = '\f', n = '\n', r = '\r', t = '\t' }

function M.decode(text, opts)
  opts = opts or {}
  local array_mt = opts.mark_arrays and require('wzt.resolve').array_mt or nil
  local null = opts.null
  local pos = 1

  local function fail(msg)
    error(string.format('json: %s at byte %d', msg, pos), 0)
  end

  local function skip()
    pos = text:find('[^ \t\r\n]', pos) or (#text + 1)
  end

  local value

  local function string_value()
    pos = pos + 1 -- opening quote
    local out = {}
    while true do
      local s, e = text:find('["\\]', pos)
      if not s then
        fail('unterminated string')
      end
      out[#out + 1] = text:sub(pos, s - 1)
      if text:sub(s, s) == '"' then
        pos = e + 1
        break
      end
      local c = text:sub(s + 1, s + 1)
      if c == 'u' then
        local cp = tonumber(text:sub(s + 2, s + 5), 16) or fail('bad \\u escape')
        pos = s + 6
        if cp >= 0xD800 and cp <= 0xDBFF and text:sub(pos, pos + 1) == '\\u' then
          local lo = tonumber(text:sub(pos + 2, pos + 5), 16) or fail('bad surrogate')
          cp = 0x10000 + (cp - 0xD800) * 0x400 + (lo - 0xDC00)
          pos = pos + 6
        end
        out[#out + 1] = utf8_char(cp)
      else
        out[#out + 1] = ESC[c] or fail('bad escape')
        pos = s + 2
      end
    end
    return table.concat(out)
  end

  function value()
    skip()
    local c = text:sub(pos, pos)
    if c == '{' then
      pos = pos + 1
      local obj = {}
      skip()
      if text:sub(pos, pos) == '}' then
        pos = pos + 1
        return obj
      end
      while true do
        skip()
        if text:sub(pos, pos) ~= '"' then
          fail('expected object key')
        end
        local k = string_value()
        skip()
        if text:sub(pos, pos) ~= ':' then
          fail("expected ':'")
        end
        pos = pos + 1
        obj[k] = value()
        skip()
        local d = text:sub(pos, pos)
        pos = pos + 1
        if d == '}' then
          return obj
        elseif d ~= ',' then
          fail("expected ',' or '}'")
        end
      end
    elseif c == '[' then
      pos = pos + 1
      local arr = {}
      skip()
      if text:sub(pos, pos) == ']' then
        pos = pos + 1
        return array_mt and setmetatable(arr, array_mt) or arr
      end
      while true do
        local v = value()
        if v ~= nil then
          arr[#arr + 1] = v
        end
        skip()
        local d = text:sub(pos, pos)
        pos = pos + 1
        if d == ']' then
          return array_mt and setmetatable(arr, array_mt) or arr
        elseif d ~= ',' then
          fail("expected ',' or ']'")
        end
      end
    elseif c == '"' then
      return string_value()
    elseif text:sub(pos, pos + 3) == 'true' then
      pos = pos + 4
      return true
    elseif text:sub(pos, pos + 4) == 'false' then
      pos = pos + 5
      return false
    elseif text:sub(pos, pos + 3) == 'null' then
      pos = pos + 4
      return null
    end
    local num = text:match('^-?%d+%.?%d*[eE]?[+-]?%d*', pos)
    if not num or num == '' then
      fail('unexpected character ' .. c)
    end
    pos = pos + #num
    return tonumber(num) or fail('bad number')
  end

  local v = value()
  skip()
  if pos <= #text then
    fail('trailing data')
  end
  return v
end

return M
