-- trip_panel.lua: a panel with the line, the speed, the next stop and how late the bus is,
-- and two buttons. Copy it into the `plugins` folder next to the game (see docs/PLUGINS.md,
-- "On-screen panels"). F10 gives the panel the mouse, Esc (or F10 again) gives it back.

-- an older openOMSI has no panels: the same news as a line of text
if not (omsi.ui and omsi.ui.version >= 1) then
  omsi.every(5, function()
    local i = omsi.info()
    if i.next_stop then omsi.message("Next stop: " .. i.next_stop, 4) end
  end)
  return
end

local compact = omsi.data.compact or false

local function delay_badge(seconds)
  if not seconds then return { type = "badge", text = "no duty", color = "#3E3E3E", text_color = "#C8C8C8" } end
  local minutes = math.floor(math.abs(seconds) / 60 + 0.5)
  if minutes == 0 then return { type = "badge", text = "on time", color = "#2E7D32" } end
  local late = seconds > 0
  return { type = "badge", text = (late and "+" or "-") .. minutes .. " min", color = late and "#C62828" or "#1565C0" }
end

local function show()
  local i = omsi.info()
  if not omsi.has_vehicle() then
    omsi.ui.remove("trip")
    return
  end
  local children = {
    { type = "row", children = {
      { type = "icon", name = "directions_bus", color = "#F47F30" },
      { type = "text", text = i.line and ("Line " .. i.line) or omsi.vehicle() or "", weight = "bold", grow = true, wrap = false },
      delay_badge(i.line and i.delay),
    } },
  }
  if not compact then
    children[#children + 1] = { type = "row", gap = 6, children = {
      { type = "icon", name = "location_on", size = 16, color = "#8E8E8E" },
      { type = "text", text = i.next_stop or "-", grow = true },
    } }
    if i.stops and i.next_stop_number then
      children[#children + 1] = { type = "bar", value = (i.next_stop_number - 1) / math.max(i.stops - 1, 1) }
    end
    children[#children + 1] = { type = "row", gap = 6, children = {
      { type = "icon", name = "speed", size = 16, color = "#8E8E8E" },
      { type = "text", text = string.format("%.0f km/h", i.speed or 0), size = 13, color = "#C8C8C8", grow = true },
      { type = "icon", name = "group", size = 16, color = "#8E8E8E" },
      { type = "text", text = tostring(i.passengers or 0), size = 13, color = "#C8C8C8" },
    } }
  end
  children[#children + 1] = { type = "row", gap = 8, children = {
    { type = "button", id = "horn", text = "Horn", icon = "campaign", grow = true },
    { type = "button", id = "size", icon = compact and "expand_more" or "expand_less" },
  } }
  local ok, why = omsi.ui.set("trip", {
    anchor = "bottom_left", x = 16, y = 16, width = 300, accent = "#F47F30", children = children,
  })
  if not ok then omsi.warn("trip panel: " .. why) end
end

omsi.every(0.5, show)
omsi.on("vehicle", show)

omsi.on("key", function(key, down)
  if key == "F10" and down then omsi.ui.focus(not omsi.ui.focused()) end
end)

omsi.on("ui_click", function(panel, element)
  if element == "horn" then
    omsi.trigger("horn")
  elseif element == "size" then
    compact = not compact
    omsi.data.compact = compact
    show()
  end
end)

omsi.on("next_stop", function(stop)
  omsi.ui.toast(stop, { title = "Next stop", icon = "location_on", seconds = 4 })
end)
