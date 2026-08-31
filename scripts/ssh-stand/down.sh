#!/usr/bin/env bash
# Погасить стенд. Ключ и тома удаляем: стенд одноразовый по замыслу.
set -euo pipefail
cd "$(dirname "$0")"
docker compose down -v --remove-orphans
rm -rf .stand
