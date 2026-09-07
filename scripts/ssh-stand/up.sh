#!/usr/bin/env bash
# Поднять стенд и напечатать переменные, которые ждут тесты.
#
# Ключ создаётся здесь, а не лежит в репозитории: приватных ключей в git быть не должно,
# даже тестовых - секрет-сканеры на них справедливо ругаются, а для проекта, который
# продаёт безопасность, это лишний разговор.
set -euo pipefail
cd "$(dirname "$0")"

mkdir -p .stand
if [ ! -f .stand/id_ed25519 ]; then
  ssh-keygen -t ed25519 -N '' -C 'serein-ssh-stand' -f .stand/id_ed25519 >/dev/null
  cp .stand/id_ed25519.pub .stand/authorized_keys
fi
chmod 600 .stand/id_ed25519

docker compose up -d --build

# Ждём именно готовности принимать соединения: `up -d` возвращается раньше, чем sshd
# успевает сгенерировать ключи хоста, и первый же тест ловил бы отказ на ровном месте.
for port in 2201 2202 2203 2204 2205; do
  echo -n "жду 127.0.0.1:$port "
  for _ in $(seq 1 60); do
    if printf '' 2>/dev/null >/dev/tcp/127.0.0.1/"$port"; then echo "- готов"; break; fi
    echo -n .
    sleep 1
  done
done

# Базы данных наружу портов не публикуют, поэтому их готовность спрашиваем у самого
# compose. MySQL 8 при первом запуске создаёт системные таблицы десятки секунд, и без
# этого ожидания тесты стартовали бы раньше, чем база начнёт отвечать.
for svc in mariadb mysql web ldap; do
  echo -n "жду $svc "
  for _ in $(seq 1 90); do
    state=$(docker compose ps --format '{{.Health}}' "$svc" 2>/dev/null | head -1)
    if [ "$state" = "healthy" ]; then echo "- готов"; break; fi
    echo -n .
    sleep 2
  done
done

# Ветку каталога образ сам не заводит - создаём её и наполняем примерами. Команда
# идемпотентная по смыслу: на повторном запуске она откажет, и это нормально, ветка уже
# есть. Без неё поиск возвращал бы «No such object», и тест проверял бы пустоту.
docker compose exec -T ldap dsconf localhost backend create \
  --suffix dc=probe,dc=local --be-name userRoot --create-suffix --create-entries \
  >/dev/null 2>&1 || true

cat <<VARS

Стенд поднят. Переменные для тестов:

  export SEREIN_STAND_HOST=127.0.0.1
  export SEREIN_STAND_DEBIAN_PORT=2201
  export SEREIN_STAND_ALPINE_PORT=2202
  export SEREIN_STAND_USER=probe
  export SEREIN_STAND_PASSWORD=probe-pass
  export SEREIN_STAND_KEY=$(pwd)/.stand/id_ed25519
  export SEREIN_STAND_ALPINE_INTERNAL=alpine
  # Базы видны только изнутри сети стенда - по именам сервисов, как и на настоящем сервере.
  export SEREIN_STAND_PG_HOST=postgres
  export SEREIN_STAND_REDIS_HOST=redis
  export SEREIN_STAND_MARIADB_HOST=mariadb
  export SEREIN_STAND_MYSQL_HOST=mysql
  export SEREIN_STAND_WEB_HOST=web
  # Каталог, в отличие от баз, доступен напрямую: клиент открывает сокет сам.
  export SEREIN_STAND_LDAP_URL=ldap://127.0.0.1:13389
  export SEREIN_STAND_LDAP_BASE=dc=probe,dc=local
  export SEREIN_STAND_HOSTKEY_PORT=2203
  export SEREIN_STAND_NOSFTP_PORT=2204
  export SEREIN_STAND_VNC_PORT=2205

Запуск: cargo test --manifest-path ../../src-tauri/Cargo.toml -- --ignored
Остановить: ./down.sh
VARS
