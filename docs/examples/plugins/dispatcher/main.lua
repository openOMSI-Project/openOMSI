-- plugins/dispatcher/main.lua: a dispatcher and a small career.
--
-- A panel (Ctrl+D shows and hides it, F10 gives it the mouse) with three tabs:
--   Duty     the duty now, the next stops with their times and the delay
--   Career   what the stops and trips earned, over every session
--   Offer    the next duties of the map's timetable, one click takes one
-- The pay is the plugin's own game: a stop served on time pays, a late one less, a crash
-- costs. Everything is kept in the plugin's storage.

local fmt, ui = omsi.fmt, omsi.ui

local settings = omsi.plugin.settings({
  { key = "pay_stop", type = "number", label = "Pay per stop", default = 1.5, min = 0, max = 10, step = 0.5 },
  { key = "late", type = "number", label = "Late after (s)", default = 120, min = 30, max = 600, step = 30 },
  { key = "currency", type = "choice", label = "Currency", choices = { "EUR", "PLN", "GBP" }, default = "EUR" },
  { key = "toasts", type = "bool", label = "A notification for each trip", default = true },
}, "Dispatcher")

local career = omsi.storage.get("career") or { money = 0, stops = 0, late = 0, trips = 0, km = 0 }
local today = { money = 0, stops = 0, late = 0, delays = {} }
local tab = 1

local function money(x) return fmt.money(x, settings.currency) end

local function save()
  omsi.storage.set("career", career)
end

local function duty_rows()
  local rows = {}
  local stops = omsi.duty.stops() or {}
  local delay = omsi.duty.delay() or 0
  for _, s in ipairs(stops) do
    if s.stops and not s.passed and #rows < 6 then
      rows[#rows + 1] = { s.name, fmt.clock(s.departure), fmt.delay(delay) }
    end
  end
  return rows
end

local function offer_rows()
  local rows = {}
  for _, line in ipairs(omsi.timetable.lines()) do
    if line.user_allowed then
      for _, tour in ipairs(line.tours) do
        if tour.today and #rows < 6 then
          rows[#rows + 1] = { line = line.name, tour = tour.number, trips = tour.trips }
        end
      end
    end
  end
  return rows
end

local function panel()
  local d = omsi.duty.get()
  local children = {
    { type = "row", children = {
      { type = "icon", name = "directions_bus", color = "#F47F30" },
      { type = "text", text = "Dispatcher", size = 16, weight = "bold", grow = true },
      { type = "badge", id = "money", text = money(career.money), color = "#2E7D32" },
    } },
    { type = "tabs", id = "tab", tabs = { "Duty", "Career", "Offer" }, selected = tab },
  }
  if tab == 1 then
    if d then
      children[#children + 1] = { type = "text", text = string.format("Line %s / %s  ·  trip %d of %d to %s", d.line, d.tour, d.trip, d.trips, d.terminus), weight = "medium" }
      children[#children + 1] = { type = "table", id = "stops", columns = { "Next stops", "Due", "Delay" }, widths = { 3, 1, 1 }, rows = duty_rows() }
      children[#children + 1] = { type = "chart", id = "delays", values = today.delays, height = 36, fill = true, min = -120, max = 300 }
      children[#children + 1] = { type = "row", children = {
        { type = "button", id = "skip", text = "Skip stop", grow = true },
        { type = "button", id = "end", text = "End duty", icon = "logout", grow = true },
      } }
    else
      children[#children + 1] = { type = "text", text = "No duty. The Offer tab lists today's.", color = "#C8C8C8" }
    end
  elseif tab == 2 then
    local function line(a, b) return { type = "row", align = "between", children = { { type = "text", text = a }, { type = "text", text = b, weight = "bold" } } } end
    children[#children + 1] = line("Today", money(today.money))
    children[#children + 1] = line("Stops today (late)", string.format("%d (%d)", today.stops, today.late))
    children[#children + 1] = { type = "divider" }
    children[#children + 1] = line("All sessions", money(career.money))
    children[#children + 1] = line("Stops / trips", string.format("%d / %d", career.stops, career.trips))
    children[#children + 1] = line("Punctuality", career.stops > 0 and string.format("%d %%", math.floor(100 * (1 - career.late / career.stops))) or "-")
    children[#children + 1] = { type = "button", id = "settings", text = "Settings", icon = "settings" }
  else
    for i, o in ipairs(offer_rows()) do
      children[#children + 1] = { type = "button", id = "take" .. i, text = string.format("Line %s, tour %s (%d trips)", o.line, o.tour, o.trips) }
    end
  end
  return { anchor = "top_left", x = 16, y = 80, width = 360, accent = "#F47F30", draggable = true, children = children }
end

local shown = true
local function refresh()
  if shown then ui.set("main", panel()) end
end

omsi.on("ui_change", function(p, element, value)
  if element == "tab" then tab = value; refresh() end
end)

omsi.on("ui_click", function(p, element)
  if element == "skip" then omsi.duty.skip_stop()
  elseif element == "end" then omsi.duty.finish()
  elseif element == "settings" then omsi.plugin.show_settings(true)
  elseif element and element:match("^take%d") then
    local o = offer_rows()[tonumber(element:sub(5))]
    if o then
      local ok, why = omsi.duty.start(o.line, o.tour)
      ui.toast(ok and ("Duty taken: line " .. o.line) or ("No duty: " .. tostring(why)), { icon = ok and "check" or "warning" })
      tab = 1
    end
  end
  refresh()
end)

omsi.on("stop_depart", function(name, id, number, delay)
  local late = delay > settings.late
  local pay = settings.pay_stop * (late and 0.5 or 1)
  today.money, career.money = today.money + pay, career.money + pay
  today.stops, career.stops = today.stops + 1, career.stops + 1
  if late then today.late, career.late = today.late + 1, career.late + 1 end
  today.delays[#today.delays + 1] = delay
  if #today.delays > 40 then table.remove(today.delays, 1) end
  save()
  refresh()
end)

omsi.on("trip_done", function(trip, how, driving, comfort, tickets)
  career.trips = career.trips + 1
  local bonus = (driving + comfort) / 100
  career.money, today.money = career.money + bonus, today.money + bonus
  save()
  if settings.toasts then
    ui.toast(string.format("Trip %d %s: +%s", trip, how, money(bonus)), { title = "Dispatcher", icon = "payments", color = "#2E7D32" })
  end
end)

omsi.on("crash", function(kj)
  local cost = math.min(50, kj / 10)
  career.money, today.money = career.money - cost, today.money - cost
  save()
  ui.toast("Crash: -" .. money(cost), { icon = "warning", color = "#C62828" })
end)

omsi.on("setting", function(key, value) settings[key] = value; refresh() end)

omsi.input.hotkey("Ctrl+KeyD", function()
  shown = not shown
  if shown then refresh() else ui.remove("main") end
end)
omsi.input.hotkey("F10", function() ui.focus(not ui.focused()) end)

omsi.every(1, refresh)
omsi.on("stop", save)
refresh()
