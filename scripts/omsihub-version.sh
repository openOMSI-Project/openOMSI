#!/bin/sh
# Print the version of this fork's checked-out commit: MAJOR.MINOR.COMMIT.OURS.
#   MAJOR.MINOR.COMMIT  the openOMSI version it is on: scripts/version.sh's number at the
#                       last commit of the project's main merged into it
#   OURS                the fork's own commits on top (the Omsi-Hub interface, merges of
#                       the project's main included)
# Both only grow, so every release of the fork is newer than the one before to the updater
# (crate::updater::newer compares the numbers in order). Needs the full history and the
# project's main as `upstream/main` (CI: fetch-depth: 0 and a fetch of upstream).
set -eu
cd "$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
up=${UPSTREAM_REF:-upstream/main}
mb=$(git merge-base HEAD "$up")
base=$(git show "$mb:VERSION" | tr -d ' \r\n')
last=$(git log -1 --format=%H "$mb" -- VERSION 2>/dev/null || true)
if [ -n "$last" ]; then n=$(git rev-list --count --invert-grep --fixed-strings --grep="[skip ci]" --grep="[skip actions]" "$last..$mb"); else n=0; fi
ours=$(git rev-list --count "$up..HEAD")
echo "$base.$n.$ours"
