-- plugins/weather_controller.lua: the weather and the clock at hand.
--
-- Ctrl+W shows a panel (and gives it the mouse): the weather files of the game as tabs,
-- sliders for the temperature, the visibility and how hard it rains or snows, the snow
-- cover, and buttons that move the clock. "Day cycle" makes the weather follow the hour on
-- its own: fog in the early morning, showers in the afternoon.

local ui = omsi.ui
local shown = false
local cycle = omsi.storage.get("cycle") or false

local function presets()
  local list = omsi.weather.presets()
  local names, files = { "Map" }, { "" }
  for i = 1, math.min(#list, 4) do
    names[#names + 1] = list[i].name
    files[#files + 1] = list[i].file
  end
  return names, files
end

local function panel()
  local w = omsi.weather.get()
  if not w then return nil end
  local names = presets()
  local kind = w.precipitation
  return { anchor = "center", width = 380, draggable = true, accent = "#4FC3F7", children = {
    { type = "row", children = {
      { type = "icon", name = "cloud", color = "#4FC3F7" },
      { type = "text", text = "Weather " .. omsi.fmt.clock(omsi.world.time() or 0), size = 16, weight = "bold", grow = true },
      { type = "button", id = "close", icon = "close" },
    } },
    { type = "tabs", id = "preset", tabs = names, selected = 1 },
    { type = "row", align = "between", children = { { type = "text", text = "Temperature" }, { type = "badge", id = "t_value", text = string.format("%.0f °C", w.temperature) } } },
    { type = "slider", id = "temperature", min = -20, max = 40, step = 1, value = w.temperature },
    { type = "row", align = "between", children = { { type = "text", text = "Visibility" }, { type = "badge", id = "v_value", text = omsi.fmt.distance(w.visibility) } } },
    { type = "slider", id = "visibility", min = 100, max = 20000, step = 100, value = w.visibility },
    { type = "tabs", id = "precipitation", tabs = { "Dry", "Rain", "Snow" }, selected = kind == "rain" and 2 or kind == "snow" and 3 or 1 },
    { type = "slider", id = "rate", min = 0, max = 1, step = 0.05, value = w.precipitation_rate },
    { type = "checkbox", id = "snow_cover", text = "Snow lies", checked = w.snow_cover },
    { type = "checkbox", id = "cycle", text = "Day cycle", checked = cycle },
    { type = "row", children = {
      { type = "button", id = "earlier", text = "-1 h", grow = true },
      { type = "button", id = "morning", text = "07:00", grow = true },
      { type = "button", id = "later", text = "+1 h", grow = true },
    } },
    { type = "text", id = "note", text = w.locked and "The weather follows a real station (METAR): it cannot be changed." or "", size = 12, color = "#C8C8C8" },
  } }
end

local function show(on)
  shown = on
  if on then
    local p = panel()
    if p then ui.set("weather", p) end
  else
    ui.remove("weather")
  end
  ui.focus(on)
end

local function set(values)
  local ok, why = omsi.weather.set(values)
  if not ok then ui.toast(why, { icon = "warning" }) end
end

omsi.on("ui_change", function(p, element, value)
  if p ~= "weather" then return end
  if element == "preset" then
    local _, files = presets()
    local ok, why = omsi.weather.preset(files[value] or "", 20)
    if not ok then ui.toast(why, { icon = "warning" }) end
  elseif element == "temperature" then
    set({ temperature = value })
    ui.update("weather", "t_value", { text = string.format("%.0f °C", value) })
  elseif element == "visibility" then
    set({ visibility = value })
    ui.update("weather", "v_value", { text = omsi.fmt.distance(value) })
  elseif element == "precipitation" then
    set({ precipitation = ({ "none", "rain", "snow" })[value] })
  elseif element == "rate" then
    set({ precipitation_rate = value })
  elseif element == "snow_cover" then
    set({ snow_cover = value })
  elseif element == "cycle" then
    cycle = value
    omsi.storage.set("cycle", cycle)
  end
end)

omsi.on("ui_click", function(p, element)
  if p ~= "weather" then return end
  local t = omsi.world.time() or 0
  if element == "close" then show(false)
  elseif element == "earlier" then omsi.world.set_time(t - 3600)
  elseif element == "later" then omsi.world.set_time(t + 3600)
  elseif element == "morning" then omsi.world.set_time("07:00")
  end
end)

-- the day cycle: what the hour brings
omsi.on("hour", function(h)
  if not cycle then return end
  if h == 5 then set({ visibility = 400, precipitation = "none" })
  elseif h == 9 then set({ visibility = 15000 })
  elseif h == 15 then set({ precipitation = "rain", precipitation_rate = 0.4 })
  elseif h == 17 then set({ precipitation = "none" })
  end
end)

omsi.on("weather", function(name)
  omsi.log("the weather changed: " .. tostring(name))
  if shown then show(true) end
end)

omsi.input.hotkey("Ctrl+KeyW", function() show(not shown) end)
