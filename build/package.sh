#!/bin/sh
# Run on the build machine from the repo root, after build/app.sh (Linux) and the Windows app build:
#   CARGO_TARGET_DIR=target-win/app cargo build --release --target x86_64-pc-windows-gnu --manifest-path app/Cargo.toml
# Packs what a person needs to try it into dist/: a zip for Windows (the program, WebView2Loader.dll beside it, the
# instructions) and a tar.gz for Linux (the program, the instructions). The version is the app's (app/Cargo.toml)
# plus the date, so two tries the same day are told apart by the time.
set -e
V=$(sed -n 's/^version = "\(.*\)"/\1/p' app/Cargo.toml | head -1)
STAMP=$(date +%Y%m%d-%H%M)
WIN=target-win/app/x86_64-pc-windows-gnu/release
LIN=target-u2404/app/release
mkdir -p dist
for f in "$WIN/stream-mic.exe" "$WIN/WebView2Loader.dll" "$LIN/stream-mic"; do
  [ -f "$f" ] || { echo "missing $f: build first"; exit 1; }
done

T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
# the licenses: ours, the notice, every crate's text for each platform
python3 tools/third_party.py app/Cargo.toml "$T/tp-win.txt" x86_64-pc-windows-gnu
python3 tools/third_party.py app/Cargo.toml "$T/tp-linux.txt" x86_64-unknown-linux-gnu
# Windows
d="$T/stream-mic-$V-windows"
mkdir -p "$d"
cp "$WIN/stream-mic.exe" "$WIN/WebView2Loader.dll" LICENSE NOTICE "$d/"
cp "$T/tp-win.txt" "$d/THIRD_PARTY_LICENSES.txt"
# VB-CABLE: VB-Audio's driver pack zip, unchanged (their bundling terms: as is, with the origin and the donationware
# line, which NOTICE and the instructions carry). Put the downloaded zip in build/vendor/ (not in git).
VB=$(ls build/vendor/VBCABLE_Driver_Pack*.zip 2>/dev/null | tail -1)
[ -n "$VB" ] && cp "$VB" "$d/" || echo "note: no build/vendor/VBCABLE_Driver_Pack*.zip, the Windows zip goes without VB-CABLE"
# Notepad on older Windows wants CRLF; the BOM tells it UTF-8
printf '\357\273\277' > "$d/使用說明.txt"
sed 's/$/\r/' build/dist/windows.txt >> "$d/使用說明.txt"
(cd "$T" && zip -qr9 "$OLDPWD/dist/stream-mic-$V-windows-$STAMP.zip" "stream-mic-$V-windows")
# Linux
d="$T/stream-mic-$V-linux"
mkdir -p "$d"
cp "$LIN/stream-mic" LICENSE NOTICE "$d/"
cp "$T/tp-linux.txt" "$d/THIRD_PARTY_LICENSES.txt"
cp build/dist/linux.txt "$d/使用說明.txt"
tar -C "$T" -czf "dist/stream-mic-$V-linux-$STAMP.tar.gz" "stream-mic-$V-linux"
ls -l dist/*"$STAMP"*
