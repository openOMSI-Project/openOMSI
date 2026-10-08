#!/bin/sh
# Start the openOMSI dedicated server: start.sh /path/to/OMSI2 [server.cfg]
# (the OMSI 2 folder: a complete original installation, as the players have it; mods
# go into this folder's Vehicles, maps, Sceneryobjects … as in the game's content folder)
# Exit code 75 = restart from the local dispatch page; any other code ends the loop.
set -u
here="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
root="${1:?usage: start.sh /path/to/OMSI2 [server.cfg]}"
cfg="${2:-$here/server.cfg}"
set +e
while true; do
  if [ -n "${BASH_VERSION-}" ]; then
    # shellcheck disable=SC2039
    "$here/openomsi" --root "$root" --server "$cfg" 2>&1 | tee -a "$here/server.log"
    code=${PIPESTATUS[0]}
  else
    "$here/openomsi" --root "$root" --server "$cfg" >>"$here/server.log" 2>&1
    code=$?
  fi
  if [ "$code" -eq 75 ]; then
    echo "server: restarting..." >&2
    continue
  fi
  exit "$code"
done
