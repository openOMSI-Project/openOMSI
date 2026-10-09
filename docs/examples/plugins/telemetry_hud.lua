-- plugins/telemetry_hud.lua: a driver's HUD and a telemetry feed.
--
-- Bottom right: the speed with its last minute as a chart, the gear, rpm, fuel, the
-- passengers, the speed limit and the traffic light ahead. With the setting "Send" on, the
-- same values go twice a second as JSON to a program on this computer (UDP, the port of
-- the settings) - a dashboard on a second screen, a stream overlay.

local fmt, ui = omsi.fmt, omsi.ui

local settings = omsi.plugin.settings({
  { key = "send", type = "bool", label = "Send the values to a program on this computer", default = false },
  { key = "port", type = "number", label = "Its UDP port", default = 47800, min = 1024, max = 65535, step = 1 },
  { key = "unit", type = "choice", label = "Speed in", choices = { "km/h", "mph" }, default = "km/h" },
}, "Telemetry HUD")

local history = {}
local warned_light = nil

local function speed_text(kmh)
  return settings.unit == "mph" and fmt.speed(kmh, "mph") or fmt.speed(kmh)
end

local function light_badge()
  local l = omsi.traffic.light_ahead(120)
  if not l then return { type = "badge", text = "no light", color = "#3E3E3E" } end
  local colors = { red = "#C62828", red_yellow = "#F9A825", yellow = "#F9A825", green = "#2E7D32", green_yellow = "#9E9D24", dark = "#3E3E3E" }
  return { type = "badge", text = string.format("%s %s", l.aspect, fmt.distance(l.distance)), color = colors[l.aspect] }
end

local function show()
  local s = omsi.bus.state()
  if not s then ui.remove("hud"); return end
  local limit = omsi.map.speed_limit()
  local fuel = s.fuel and math.min(1, s.fuel / 250) or 0
  ui.set("hud", { anchor = "bottom_right", x = 16, y = 16, width = 300, children = {
    { type = "row", align = "between", children = {
      { type = "text", text = speed_text(math.abs(s.speed)), size = 28, weight = "bold" },
      { type = "badge", text = limit and ("limit " .. speed_text(limit)) or "no limit", color = (limit and math.abs(s.speed) > limit + 3) and "#C62828" or "#3E3E3E" },
    } },
    { type = "chart", values = history, height = 40, min = 0, fill = true },
    { type = "row", align = "between", children = {
      { type = "text", text = "Gear " .. tostring(s.gear or "-") },
      { type = "text", text = s.rpm and string.format("%d rpm", math.floor(s.rpm + 0.5)) or "" },
      { type = "text", text = string.format("%d aboard", s.passengers or 0) },
    } },
    { type = "row", children = { { type = "icon", name = "water_drop", size = 16 }, { type = "bar", value = fuel, grow = true, color = fuel < 0.15 and "#C62828" or nil } } },
    light_badge(),
  } })
end

omsi.every(0.25, function()
  local v = omsi.bus.velocity()
  if v then
    history[#history + 1] = math.abs(v)
    if #history > 240 then table.remove(history, 1) end
  end
end)
omsi.every(0.5, show)

omsi.every(0.5, function()
  if not settings.send then return end
  local s = omsi.bus.state()
  local x, y, z, heading = omsi.position()
  local d = omsi.duty.get()
  local ok, why = omsi.send(math.floor(settings.port), omsi.json.encode({
    time = omsi.world.time(), speed = s and s.speed, gear = s and s.gear, rpm = s and s.rpm,
    x = x, y = y, heading = heading, passengers = s and s.passengers,
    line = d and d.line, next_stop = d and d.next_stop and d.next_stop.name, delay = d and d.delay,
  }))
  if not ok then omsi.warn("not sent: " .. tostring(why)) end
end)

-- a warning when the light ahead turns red close by
omsi.on("light_ahead", function(aspect, distance)
  if aspect == "red" and distance and distance < 60 and warned_light ~= distance then
    warned_light = distance
    ui.toast("Red light ahead", { icon = "warning", color = "#C62828", seconds = 3 })
  end
end)
omsi.on("red_light", function(kmh)
  ui.toast(string.format("Red light run at %s", speed_text(kmh)), { icon = "warning", color = "#C62828" })
end)
omsi.on("setting", function(key, value) settings[key] = value end)
omsi.input.hotkey("Ctrl+KeyT", function() omsi.plugin.show_settings(true); ui.focus(true) end)
