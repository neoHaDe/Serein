#!/usr/bin/env bash
# Сборка RPM для Fedora и родственных дистрибутивов - ВНУТРИ контейнера Fedora.
#
# Почему в контейнере, а не на сборочной машине с Debian: собранный бинарь требует ту
# glibc, с которой его слинковали. Пакет, сделанный на Debian, на Fedora поставится и
# не запустится - а выглядеть это будет как «программа сломана», а не как «пакет собран
# не там». Поэтому среда сборки должна совпадать с целевой.
#
# Запуск (на машине с Docker):
#   docker run --rm -v "$PWD":/src -w /src registry.fedoraproject.org/fedora:43 \
#     bash scripts/build-fedora.sh
#
# Три вещи, выясненные на живой сборке, чтобы не выяснять заново:
#
#   1. Образ берётся из registry.fedoraproject.org, а не из Docker Hub: Hub с домашнего
#      сервера недоступен вовсе - уходит в IPv6 и получает «network is unreachable».
#      Заодно это официальный реестр самой Fedora, что тут и уместнее.
#   2. Rust ставится из репозитория дистрибутива, а не через rustup: static.rust-lang.org
#      с того же сервера не резолвится. Подробности ниже, у самой установки.
#   3. Контейнер работает от root и оставляет файлы, которые обычный пользователь на
#      хосте удалить не сможет. Чистить target тоже контейнером:
#        docker run --rm -v "$PWD":/src -w /src <образ> rm -rf src-tauri/target/release/bundle/rpm
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

echo "== системные пакеты =="
# Список ровно тот, что нужен, и каждый по делу:
#   webkit2gtk4.1-devel     - движок окна, основа Tauri
#   gtk3-devel              - окно и меню
#   libayatana-appindicator-gtk3-devel - значок в трее
#   systemd-devel           - libudev, через него serialport видит COM-порты
#   openssl-devel           - native-tls (сертификаты и HTTPS-утилита)
#   rpm-build, patchelf     - упаковка
# Репозиторий Cisco отключён намеренно: оттуда идёт только `openh264` для H.264, который
# нам не нужен, а его зеркала регулярно недоступны и роняют всю сборку. Без него dnf берёт
# заглушку `noopenh264` из основного репозитория.
dnf -y install --setopt=install_weak_deps=False --disablerepo='fedora-cisco-openh264' \
  gcc gcc-c++ make git file which \
  webkit2gtk4.1-devel gtk3-devel libayatana-appindicator-gtk3-devel \
  systemd-devel openssl-devel \
  rpm-build patchelf nodejs npm >/dev/null

echo "== rust =="
# Тулчейн берём из репозитория Fedora, а не через rustup. Две причины, и обе выяснились
# на живой сборке: static.rust-lang.org с нашего сервера не резолвится вовсе, а в самой
# Fedora лежит вполне свежий Rust - на 43 это 1.98. Заодно и честнее: пакет для
# дистрибутива собирается его же тулчейном.
#
# Образ приходится обновлять вслед за зависимостями: на 41 с её Rust 1.91 сборка
# останавливается на IronRDP, которому нужен 1.94.
dnf -y install --disablerepo='fedora-cisco-openh264' rust cargo >/dev/null
cargo --version

echo "== помощник RDP =="
# Отдельная программа поставки, и Tauri проверяет её наличие ещё на этапе сборки: без
# файла не собирается ничего. Собирается тем же тулчейном, что и приложение, поэтому
# glibc у них совпадает - ради этого весь контейнер и затевался.
bash scripts/build-rdp-helper.sh

echo "== зависимости фронтенда =="
[ -d node_modules ] || npm ci
npm run typecheck

echo "== сборка =="
npm run tauri -- build --config src-tauri/tauri.fedora.conf.json

echo
echo "готовые пакеты:"
find src-tauri/target/release/bundle -maxdepth 3 -type f -name '*.rpm' -print
