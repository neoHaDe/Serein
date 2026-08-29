#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
export PATH="${HOME}/.cargo/bin:${PATH}"
echo "tsc --noEmit"
npm run typecheck
echo "cargo check --locked"
cargo check --manifest-path src-tauri/Cargo.toml --locked
echo "cargo test --lib --locked"
cargo test --manifest-path src-tauri/Cargo.toml --lib --locked
echo "smoke ok"