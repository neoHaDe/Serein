#!/usr/bin/env bash
# Собирает помощника RDP и кладёт его туда, откуда Tauri заберёт его в поставку.
#
# Отдельный шаг, а не часть основной сборки, потому что помощник - отдельная рабочая
# область со своим `Cargo.lock`. Это не прихоть: крейты одной рабочей области делят одно
# разрешение зависимостей, а IronRDP через `picky` прибивает шестнадцать крейтов
# RustCrypto к release candidate'ам, с которыми SSH-ядро приложения не сходится. Слить их
# в одну область - значит вернуть конфликт, ради обхода которого всё и затевалось.
#
# Tauri ищет внешний бинарь по имени с суффиксом целевой тройки, поэтому копия
# переименовывается. Имя после установки - без суффикса, ровно то, которое ждёт `rdp.rs`.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root/rdp-helper"

echo "== собираю помощника RDP =="
cargo build --release

triple="$(rustc -vV | awk '/^host:/ {print $2}')"
ext=""
case "$triple" in *windows*) ext=".exe";; esac

src="target/release/serein-rdp$ext"
dest_dir="$root/src-tauri/binaries"
dest="$dest_dir/serein-rdp-$triple$ext"

mkdir -p "$dest_dir"
cp "$src" "$dest"

printf "готово: %s (%s байт)\n" "${dest#$root/}" "$(wc -c < "$dest")"
