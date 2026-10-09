# Tomorrow.io weather areas

Players can choose **Tomorrow.io** on the launcher's weather card, or pass
`--weather tomorrow`, to follow the nearest configured map area while driving.
Maps contain locations; each player supplies their own private API key.

## Players

1. Get a Tomorrow.io API key for your account and save it as plain text in
   `~/.openomsi/tomorrow-api-key.txt`. On Windows this is
   `%USERPROFILE%\.openomsi\tomorrow-api-key.txt`.
2. Install the map author's `openomsi_weather.cfg` beside the map's `global.cfg`.
   You can also create it using the editor below. Ordinary OMSI ignores the
   additional file; `global.cfg` does not need modification.
3. Choose **Tomorrow.io**, select the map and bus, and start driving in a build
   containing this feature. The editor's availability does not mean this draft
   feature is included in the official release you have installed.

Environment overrides are `OMSI_TOMORROW_API_KEY` and `OMSI_TOMORROW_KEY_FILE`.
Never include a key in the map config, a shared preset or an issue report.
Only the configured map area's coordinates and your API key are sent to Tomorrow.io,
never the player's position.
Missing config/key or failed downloads preserve fallback/current weather and
show a message. Switching to a custom weather preset ends this mode.

## Map authors: create the config

Use the independent [community weather editor](https://openomsi.mxnticek.eu/weather/)
([English interface](https://openomsi.mxnticek.eu/en/weather/)):

1. Select your map's `global.cfg`; the editor reads it locally in your browser.
2. Create areas, assign spawnpoints and enter `latitude;longitude`. Several
   spawnpoints can share a location. With real place names, opt into grouping
   by the name before the first comma and review the assignments.
3. For map 400 v1.5.2 the website offers approximate town-centre coordinates.
   Review them when using a different version. Other maps can use manual GPS
   or the website's optional online search after explicit consent; choose the
   correct place when names are ambiguous.
4. Export `openomsi_weather.cfg` beside `global.cfg`. Reload the map after editing.

The website and its presets are maintained separately from openOMSI. The game
reads only the exported config, and does not depend on the website at runtime.
The map file stays local. Optional geocoding sends place names and the selected
country to the provider, as explained in the website's consent panel; it does
not need a Tomorrow.io key. Game X/Y coordinates are not geographical lat/lon.

## Config format

The sidecar is UTF-8 JSON, version 1. A content-root override at the same relative
`maps/<map>/openomsi_weather.cfg` path is supported. Example structure with
synthetic object/tile identifiers (use the editor to export your own):

```json
{
  "version": 1,
  "refresh_minutes": 15,
  "groups": [{
    "name": "City centre",
    "latitude": 50.0755,
    "longitude": 14.4378,
    "spawnpoints": [{"object_id": 7, "tile": [0, 0], "name": "Example"}]
  }]
}
```

Spawnpoints are identified by object ID and tile coordinates. Their positions
come from the map's existing entrypoints. The closest anchor selects its group;
a 200 m tolerance keeps the previous group near boundaries. A spawnpoint cannot
belong to multiple groups. Unassigned spawnpoints are not weather anchors.
Unknown fields, invalid GPS, empty groups and duplicate assignments are rejected.

## Requests, cache and fallback

Only the active location is fetched, using Tomorrow.io's metric Realtime endpoint.
Coordinates normalized to six decimal places share a persistent cache across
groups, maps and restarts. `refresh_minutes` is 15–1440. Private bookkeeping is
stored in `~/.openomsi/weather-cache`, separated by a key fingerprint; concurrent
instances coordinate with a file lock. Corrupt bookkeeping prevents new requests.

The local budget counts successful and failed attempts: at least 180 s between
new requests, at most 20 per rolling hour and 450 per rolling day. Check your
[account's current API allowance](https://support.tomorrow.io/hc/en-us/articles/20273728362644-Free-API-Plan-Rate-Limits);
other applications using the same key are outside this bookkeeping. HTTP 429 or
rejected credentials pause requests for an hour. Requests run outside the frame
thread, have an 8 s timeout and do not follow redirects. The key is sent in the
`apikey` header and is excluded from configs, cache contents and error messages.

Weather transitions blend over 60 simulation seconds through the same apply step
as METAR, updating ambient temperature and the snow state. The adapter passes
measured precipitation rates, visibility and cloud cover to the weather model;
the model determines visual intensity, and the existing surface model wets and
dries roads over time. Falling snow does not establish historical road snow depth.
Tomorrow.io takes precedence over METAR sync. A LAN client follows the host and
does not fetch weather. Dedicated-server operation and LAN-host startup still
need integration testing; this feature is intended for driving with a player bus.
