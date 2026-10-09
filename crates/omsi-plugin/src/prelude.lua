-- The Lua side of the `omsi` table: the saved data. Everything else - the functions, the
-- events, timers and watches - is the API registry's (src/api/), bound by src/lua.rs.
local omsi = omsi

-- saved data: omsi.data is written on the way out and read back on the next start
local function dump(v, indent, seen)
  local t = type(v)
  if t == "string" then return string.format("%q", v) end
  if t == "number" then
    if v ~= v then return "0/0" end
    if v == math.huge then return "math.huge" end
    if v == -math.huge then return "-math.huge" end
    return string.format("%.17g", v)
  end
  if t == "boolean" or t == "nil" then return tostring(v) end
  if t ~= "table" then return "nil" end
  if seen[v] then return "nil" end
  seen[v] = true
  local keys = {}
  for k in pairs(v) do
    local kt = type(k)
    if kt == "string" or kt == "number" or kt == "boolean" then keys[#keys + 1] = k end
  end
  table.sort(keys, function(a, b)
    if type(a) == type(b) then return a < b end
    return type(a) < type(b)
  end)
  local inner = indent .. "  "
  local out = { "{\n" }
  for _, k in ipairs(keys) do
    local ks
    if type(k) == "string" and k:match("^[%a_][%w_]*$") then ks = k
    else ks = "[" .. dump(k, inner, seen) .. "]" end
    out[#out + 1] = inner .. ks .. " = " .. dump(v[k], inner, seen) .. ",\n"
  end
  out[#out + 1] = indent .. "}"
  seen[v] = nil
  return table.concat(out)
end

function omsi._save()
  if next(omsi.data) == nil then
    omsi._write_data(nil)
  else
    omsi._write_data("return " .. dump(omsi.data, "", {}) .. "\n")
  end
end

function omsi.save() omsi._save() end

do
  local text = omsi._read_data()
  omsi.data = {}
  if text then
    local chunk = load(text, "=saved data", "t", { math = { huge = math.huge } })
    local ok, t = pcall(chunk or error)
    if ok and type(t) == "table" then omsi.data = t end
  end
end
