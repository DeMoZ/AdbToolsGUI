#!/bin/bash
# Собирает dist/ADB Toolkit.app и dist/ADB-Toolkit-macOS.dmg. Запуск: scripts/build-macos.sh
set -euo pipefail
cd "$(dirname "$0")/.."

python3 -m venv .venv
.venv/bin/pip install -q -r requirements.txt pyinstaller==6.22.3
.venv/bin/python -m unittest discover -s tests -t .

version=$(.venv/bin/python -c "import adbtoolkit; print(adbtoolkit.__version__)")
rm -rf build dist
.venv/bin/pyinstaller --noconfirm --windowed --name "ADB Toolkit" \
  --osx-bundle-identifier com.demoz.adbtoolkit \
  --collect-all tkinterdnd2 \
  run.py
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $version" "dist/ADB Toolkit.app/Contents/Info.plist"

staging=$(mktemp -d)
cp -R "dist/ADB Toolkit.app" "$staging/"
ln -s /Applications "$staging/Applications"
hdiutil create -quiet -volname "ADB Toolkit" -srcfolder "$staging" -ov -format UDZO dist/ADB-Toolkit-macOS.dmg
rm -rf "$staging"
echo "Готово: dist/ADB Toolkit.app, dist/ADB-Toolkit-macOS.dmg"
