#!/bin/sh
# Render the application icons from assets/icons/app/*.svg: openomsi.icns (macOS),
# openomsi.ico (Windows), the PNG window icons and the Android launcher icons. The ICO's
# 16 to 32 pixel pictures come from openomsi-small.svg (a thicker ring, a simpler bus, so
# they still read). Needs resvg (cargo install resvg), iconutil (macOS) and Python 3 with
# Pillow.
set -eu
cd "$(CDPATH= cd -- "$(dirname -- "$0")/../assets/icons/app" && pwd)"
tmp=$(mktemp -d)
mkdir "$tmp/icon.iconset"
for n in 16 32 128 256 512; do
  resvg -w "$n" openomsi-macos.svg "$tmp/icon.iconset/icon_${n}x${n}.png"
  resvg -w $((n * 2)) openomsi-macos.svg "$tmp/icon.iconset/icon_${n}x${n}@2x.png"
done
iconutil -c icns "$tmp/icon.iconset" -o openomsi.icns
for n in 48 64 128 256; do resvg -w "$n" openomsi.svg "$tmp/w$n.png"; done
for n in 16 24 32; do resvg -w "$n" openomsi-small.svg "$tmp/w$n.png"; done
cp "$tmp/w256.png" openomsi-256.png
resvg -w 512 openomsi.svg openomsi-512.png
for d in mdpi:48 hdpi:72 xhdpi:96 xxhdpi:144 xxxhdpi:192; do
  resvg -w "${d#*:}" openomsi.svg "../../../android/res/mipmap-${d%:*}/ic_launcher.png"
done
python3 - "$tmp" <<'PY'
import sys
from PIL import Image
t = sys.argv[1]
ims = [Image.open(f"{t}/w{n}.png") for n in (256, 128, 64, 48, 32, 24, 16)]
ims[0].save("openomsi.ico", sizes=[i.size for i in ims], append_images=ims[1:])
PY
rm -rf "$tmp"
echo "icons written to assets/icons/app and android/res"
