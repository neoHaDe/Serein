#!/usr/bin/env bash
# Проверка, что установленный пакет запускается и рисует окно.
# Выполняется ВНУТРИ контейнера Fedora, под виртуальным экраном.
#
# Дважды подряд эта проверка выдавала ложный приговор приложению, и оба раза виновата была
# она сама: сперва молча не поставилось графическое окружение, потом отсутствовал
# `xwininfo`, которым проверялся X-сервер. Поэтому здесь нет лишних зависимостей: живость
# X-сервера определяется по его сокету, а доказательством служит снимок экрана.
set -u

echo "=== пакет ==="
if dnf -y install /pkg/Serein-1.2.7-1.x86_64.rpm >/tmp/rpm.log 2>&1; then
  echo "пакет поставлен"
else
  echo "пакет НЕ поставился:"; tail -5 /tmp/rpm.log; exit 1
fi

echo "=== графическое окружение ==="
for p in xorg-x11-server-Xvfb dbus-x11 ImageMagick; do
  if dnf -y install "$p" >/tmp/dnf.log 2>&1; then
    echo "  $p — ок"
  else
    echo "  $p — НЕ поставился"; tail -3 /tmp/dnf.log; exit 1
  fi
done

export HOME=/tmp DISPLAY=:99 GDK_BACKEND=x11 WEBKIT_DISABLE_COMPOSITING_MODE=1

Xvfb :99 -screen 0 1280x800x24 >/tmp/xvfb.log 2>&1 &
xvfb_pid=$!
for i in $(seq 1 20); do
  [ -S /tmp/.X11-unix/X99 ] && break
  sleep 1
done
if [ -S /tmp/.X11-unix/X99 ] && kill -0 "$xvfb_pid" 2>/dev/null; then
  echo "X-сервер работает (сокет на месте)"
else
  echo "X-сервер не поднялся:"; tail -5 /tmp/xvfb.log; exit 1
fi

eval "$(dbus-launch --sh-syntax)" 2>/dev/null || echo "  (без dbus)"

echo "=== запускаю приложение ==="
serein >/tmp/app.log 2>&1 &
pid=$!
sleep 20

if kill -0 "$pid" 2>/dev/null; then
  echo "РЕЗУЛЬТАТ: процесс жив"
else
  echo "РЕЗУЛЬТАТ: процесс умер"
fi

echo "=== что приложение написало ==="
head -25 /tmp/app.log 2>/dev/null || echo "(пусто)"

echo "=== снимок экрана ==="
if import -window root /out/fedora-app.png 2>/tmp/import.log; then
  echo "снимок: $(stat -c%s /out/fedora-app.png) байт"
  # Пустой экран Xvfb — сплошной чёрный и жмётся в считанные килобайты. Нарисованное
  # окно даёт заметно больше. Это грубая, но честная проверка «что-то нарисовано».
  # Имя переменной латиницей: кириллические имена bash не принимает, и прошлый прогон
  # из-за этого объявил нарисованный экран пустым.
  size=$(stat -c%s /out/fedora-app.png)
  if [ "$size" -gt 20000 ]; then echo "на экране что-то нарисовано"; else echo "экран похож на пустой"; fi
else
  echo "снимок не вышел:"; head -3 /tmp/import.log
fi

kill "$pid" 2>/dev/null || true
echo "=== КОНЕЦ ==="
