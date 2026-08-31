#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
export PATH="${HOME}/.cargo/bin:${PATH}"
echo "tsc --noEmit"
npm run typecheck
# Без отдельного `cargo check`: он компилирует всё дерево, а следом `cargo test` делает
# то же самое под тестовым профилем. Ошибку компиляции тест ловит не хуже, просто на шаг
# позже. В CI это было половиной времени прогона.
echo "cargo test --lib --locked"
cargo test --manifest-path src-tauri/Cargo.toml --lib --locked
echo "smoke ok"