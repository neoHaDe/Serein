#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
export PATH="${HOME}/.cargo/bin:${PATH}"
need() { command -v "$1" >/dev/null 2>&1 || { echo "missing: $1" >&2; exit 1; }; }
need node; need npm; need cargo
if ! pkg-config --exists webkit2gtk-4.1 2>/dev/null; then
  echo "sudo apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev patchelf" >&2
  exit 1
fi
npm ci
npm run typecheck
npm run tauri -- build --config src-tauri/tauri.linux.conf.json
find src-tauri/target/release/bundle -maxdepth 3 -type f \( -name '*.deb' -o -name '*.AppImage' \) -print