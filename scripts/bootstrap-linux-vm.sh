#!/usr/bin/env bash
# Подготовка чистой Ubuntu/Astra VM к сборке Serein из ветки linux/port.
#
# Запуск (ветка с косой чертой в raw-ссылке не работает — берём через refs/heads):
#   curl -fsSL https://raw.githubusercontent.com/neoHaDe/Serein/refs/heads/linux/port/scripts/bootstrap-linux-vm.sh | bash
# либо, что надёжнее, склонировать репозиторий и запустить скрипт с диска.
#
# Переменные окружения:
#   SEREIN_AUTHORIZED_KEY  публичный ключ, которому разрешить вход по SSH (по умолчанию не трогаем)
#   SEREIN_REPO/BRANCH/DIR откуда и куда клонировать

set -euo pipefail

REPO="${SEREIN_REPO:-https://github.com/neoHaDe/Serein.git}"
BRANCH="${SEREIN_BRANCH:-linux/port}"
DIR="${SEREIN_DIR:-$HOME/Serein}"

echo "==> пакеты"
# На Astra `apt update` возвращает ненулевой код из-за недоступного репозитория —
# это не повод останавливать подготовку.
sudo apt update || true
sudo apt install -y openssh-server git curl pkg-config build-essential libssl-dev \
  libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev patchelf \
  libudev-dev

echo "==> доступ по SSH"
# Ключ больше не зашит в скрипт: в публичном репозитории ему не место, да и любой,
# кто запустит этот файл, пускал бы к себе на машину чужой ключ.
if [ -n "${SEREIN_AUTHORIZED_KEY:-}" ]; then
  mkdir -p ~/.ssh && chmod 700 ~/.ssh
  touch ~/.ssh/authorized_keys && chmod 600 ~/.ssh/authorized_keys
  grep -qF "$SEREIN_AUTHORIZED_KEY" ~/.ssh/authorized_keys \
    || echo "$SEREIN_AUTHORIZED_KEY" >> ~/.ssh/authorized_keys
  echo "    ключ добавлен в authorized_keys"
else
  echo "    SEREIN_AUTHORIZED_KEY не задан — authorized_keys не трогаем"
fi
sudo systemctl enable --now ssh

echo "==> rust"
if ! command -v cargo >/dev/null 2>&1; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
fi
# shellcheck disable=SC1091
source "$HOME/.cargo/env"

echo "==> node 22"
# Дистровый Node 18 оставляет npm неработающим, поэтому ставим из NodeSource
# и фиксируем: иначе очередное обновление системы вернёт сломанную версию.
if ! node -v 2>/dev/null | grep -q '^v22'; then
  curl -fsSL https://deb.nodesource.com/setup_22.x | sudo -E bash -
  sudo apt install -y nodejs
fi
sudo apt-mark hold nodejs || true

echo "==> исходники ($BRANCH)"
# Каталог не сносим: там могли остаться правки, которые ещё не уехали в репозиторий.
if [ -d "$DIR/.git" ]; then
  git -C "$DIR" fetch origin "$BRANCH"
  git -C "$DIR" checkout "$BRANCH"
  git -C "$DIR" pull --ff-only origin "$BRANCH"
elif [ -e "$DIR" ]; then
  echo "    $DIR существует и это не репозиторий — уберите его или задайте SEREIN_DIR" >&2
  exit 1
else
  git clone -b "$BRANCH" "$REPO" "$DIR"
fi
cd "$DIR"

echo "==> smoke"
npm ci
npm run smoke:linux

echo "==> сборка deb + AppImage"
./scripts/build-linux.sh

echo "==> готово"
find src-tauri/target/release/bundle -maxdepth 3 -type f \( -name '*.deb' -o -name '*.AppImage' \) -print
