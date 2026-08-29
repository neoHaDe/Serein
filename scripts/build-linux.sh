#!/usr/bin/env bash
# Сборка Linux-бандлов (.deb + AppImage) из ветки linux/port.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
export PATH="${HOME}/.cargo/bin:${PATH}"

need() { command -v "$1" >/dev/null 2>&1 || { echo "не найдено: $1" >&2; exit 1; }; }
need node; need npm; need cargo; need pkg-config

# Проверяем системные библиотеки до сборки: без них cargo падает где-то в глубине
# зависимостей, и по сообщению не понять, чего именно не хватает.
missing=()
pkg-config --exists webkit2gtk-4.1 2>/dev/null || missing+=("libwebkit2gtk-4.1-dev")
pkg-config --exists ayatana-appindicator3-0.1 2>/dev/null || missing+=("libayatana-appindicator3-dev")
# libudev тянет serialport (COM-порты). Без него сборка обрывается паникой в libudev-sys,
# и в тексте ошибки нет ни слова про то, какой пакет ставить.
pkg-config --exists libudev 2>/dev/null || missing+=("libudev-dev")
command -v patchelf >/dev/null 2>&1 || missing+=("patchelf")
if [ ${#missing[@]} -gt 0 ]; then
  echo "не хватает системных пакетов: ${missing[*]}" >&2
  echo "sudo apt install -y ${missing[*]}" >&2
  exit 1
fi

# node_modules ставим только если их нет: `npm ci` каждый раз сносит каталог целиком
# и на VM это лишние минуты на ровном месте.
[ -d node_modules ] || npm ci
npm run typecheck

npm run tauri -- build --config src-tauri/tauri.linux.conf.json

echo "готовые бандлы:"
find src-tauri/target/release/bundle -maxdepth 3 -type f \( -name '*.deb' -o -name '*.AppImage' \) -print
