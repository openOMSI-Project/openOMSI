#!/usr/bin/env bash
# The window's own frame measured without a window on the screen: a session run with
# OMSI_HIDDEN_WINDOW (never shown, frames drawn into a texture of its size) and OMSI_PROFILE,
# its exit summary as JSON for scripts/compare-performance.py.
#
#   scripts/bench-window.sh <openomsi binary> <out prefix> [plain|enhanced] [seconds] [extra args ...]
#
# The scene: Berlin-Spandau at 08:00, the MAN EN92 started (--autostart), traffic 30, the
# passengers and the timetable, the driver's view at 1280x720; at 44 s the parking brake
# is let go and the bus driven off on full throttle (it runs into the scenery a little
# later and stands there). The run ends after <seconds> (110 by default; the warm-up of
# the summary is the first 15 s of play). Writes <out prefix>.log and <out prefix>.json.
#
# Every run has a fresh, empty $HOME (default settings) and OMSI_SEED fixed, and no frame
# rate limit. Compare runs of one machine only, and several of each: the frame times move
# with whatever else the machine does; the stages' CPU times much less.
# Needs the original OMSI 2 install: $OMSI_ROOT, else "OMSI 2 Original" beside the checkout.
set -euo pipefail

repo="$(git -C "$(dirname "$0")" rev-parse --show-toplevel)"
[ $# -ge 2 ] || { sed -n '2,/^set -euo/p' "$0" | grep '^#' | sed 's/^# \{0,1\}//'; exit 2; }
bin="$1"; out="$2"; mode="${3:-plain}"; secs="${4:-110}"
shift $(( $# < 4 ? $# : 4 ))
[ -x "$bin" ] || { echo "not an executable: $bin" >&2; exit 2; }
root="${OMSI_ROOT:-$(dirname "$repo")/OMSI 2 Original}"
[ -d "$root/maps" ] || { echo "no OMSI 2 install at $root (set OMSI_ROOT)" >&2; exit 2; }
extra=()
[ "$mode" = enhanced ] && extra+=(--enhanced)
home="$(mktemp -d "${TMPDIR:-/tmp}/bench-home.XXXXXX")"
trap 'rm -rf "$home"' EXIT
mkdir -p "$home/.openomsi"
# (the swaying trees move with the wall clock)
printf 'windy_trees=0\n' >"$home/.openomsi/settings.cfg"
env HOME="$home" OMSI_SEED=1 OMSI_HIDDEN_WINDOW=1 OMSI_NO_UPDATE=1 OMSI_MAX_FPS=1000 \
  OMSI_PROFILE=1 OMSI_PROFILE_JSON="$out.json" \
  OMSI_INPUT="t=44 key .; t=45 keydown W" \
  "$bin" --root "$root" --no-menu --map maps/Berlin-Spandau/global.cfg \
  --bus Vehicles/MAN_NL_NG/MAN_EN92_main.bus --autostart --view driver \
  --traffic 30 --passengers --schedule --time 08:00 --size 1280x720 \
  --exit-after "$secs" ${extra[@]+"${extra[@]}"} ${@+"$@"} >"$out.log" 2>&1
grep -E "profile: (since|window)" "$out.log" | sed 's/^.*profile: /profile: /'
