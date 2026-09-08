#!/usr/bin/env bash
# Собирает SBOM (CycloneDX) для релиза: Rust приложения, Rust помощника RDP и npm.
#
# Зачем: список того, из чего собран бинарь, спрашивают первым при проверке цепочки
# поставки. Без него разговор начинается с «пришлите состав», а не с сути.
#
# Два файла, а не один: инструменты разные, форматы одинаковые (CycloneDX JSON), а
# склеивать их вручную - значит однажды склеить неправильно.
#
# npm берём только прод-зависимости: сборочные в артефакт не попадают, и их состав
# для потребителя бинаря шума больше, чем пользы.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

out="${1:-$root/dist/sbom}"
mkdir -p "$out"
version="$(node -p "require('./package.json').version")"

need() { command -v "$1" >/dev/null 2>&1 || { echo "не найдено: $1" >&2; exit 1; }; }
need node
need cargo

if ! cargo cyclonedx --help >/dev/null 2>&1; then
  echo "нет cargo-cyclonedx. Установить: cargo install cargo-cyclonedx --locked" >&2
  exit 1
fi

echo "Rust…"
cargo cyclonedx --manifest-path src-tauri/Cargo.toml --format json --spec-version 1.5 >/dev/null
# cargo-cyclonedx кладёт файл рядом с манифестом; переносим под понятным именем.
find src-tauri -maxdepth 1 -name '*.cdx.json' -exec mv {} "$out/serein-$version-rust.cdx.json" \;

echo "Rust: помощник RDP…"
# Помощник - отдельная рабочая область со своим замком зависимостей, и в дерево
# приложения он не попадает. В поставку попадает, поэтому и в состав должен.
cargo cyclonedx --manifest-path rdp-helper/Cargo.toml --format json --spec-version 1.5 >/dev/null
find rdp-helper -maxdepth 1 -name '*.cdx.json' -exec mv {} "$out/serein-$version-rdp-helper.cdx.json" \;

echo "npm…"
npx --yes @cyclonedx/cyclonedx-npm@latest \
  --omit dev \
  --spec-version 1.5 \
  --output-format JSON \
  --output-file "$out/serein-$version-npm.cdx.json" >/dev/null

echo "готово:"
ls -la "$out"
