#!/bin/bash
# Перерисовывает значок: assets/icon.png (1024), assets/AppIcon.icns (Mac), assets/icon.ico (Windows).
# Нужны macOS (swift, sips, iconutil) и скачанные зависимости cargo (шрифт Phosphor из egui-phosphor).
set -euo pipefail
cd "$(dirname "$0")/.."
font=$(ls ~/.cargo/registry/src/*/egui-phosphor-*/res/Phosphor-Bold.ttf | tail -1)
work=$(mktemp -d)
swift scripts/make-icon.swift "$font" assets/icon.png

mkdir "$work/AppIcon.iconset"
for s in 16 32 128 256 512; do
  sips -z $s $s assets/icon.png --out "$work/AppIcon.iconset/icon_${s}x${s}.png" >/dev/null
  sips -z $((s * 2)) $((s * 2)) assets/icon.png --out "$work/AppIcon.iconset/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$work/AppIcon.iconset" -o assets/AppIcon.icns

# .ico из PNG-картинок 16–256 (формат ICO допускает PNG внутри)
for s in 16 24 32 48 64 128 256; do sips -z $s $s assets/icon.png --out "$work/ico_$s.png" >/dev/null; done
python3 - "$work" <<'PY'
import struct, sys
work = sys.argv[1]
sizes = [16, 24, 32, 48, 64, 128, 256]
data = [open(f"{work}/ico_{s}.png", "rb").read() for s in sizes]
out = struct.pack("<HHH", 0, 1, len(sizes))
offset = 6 + 16 * len(sizes)
for s, d in zip(sizes, data):
    out += struct.pack("<BBBBHHII", s % 256, s % 256, 0, 0, 1, 32, len(d), offset)
    offset += len(d)
open("assets/icon.ico", "wb").write(out + b"".join(data))
PY
sips -z 256 256 assets/icon.png --out assets/icon-256.png >/dev/null
rm -rf "$work"
echo "Готово: assets/icon.png, assets/icon-256.png, assets/AppIcon.icns, assets/icon.ico"
