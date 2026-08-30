#!/bin/sh
# Общий старт для обоих образов.
set -e

# Ключи хоста делаем при каждом запуске заново: тесты проверяют встречу с НЕЗНАКОМЫМ
# ключом, а с ключом, вшитым в образ, эта ветка никогда бы не выполнилась.
ssh-keygen -A

# Публичный ключ приезжает томом снаружи — в репозитории приватных ключей нет и не будет.
if [ -f /keys/authorized_keys ]; then
  mkdir -p /home/probe/.ssh
  cp /keys/authorized_keys /home/probe/.ssh/authorized_keys
  chown -R probe:probe /home/probe/.ssh
  chmod 700 /home/probe/.ssh
  chmod 600 /home/probe/.ssh/authorized_keys
fi

# Пароль по SSH обычно выключен — здесь он часть проверяемой матрицы, поэтому включаем
# явно. Стенд слушает только 127.0.0.1 и живёт минуты.
{
  echo 'PasswordAuthentication yes'
  echo 'PermitRootLogin no'
  echo 'PubkeyAuthentication yes'
  # Держим и старый алгоритм, и современные: часть тестов про режим совместимости.
  echo 'HostKeyAlgorithms +ssh-rsa'
  echo 'PubkeyAcceptedAlgorithms +ssh-rsa'
} >> /etc/ssh/sshd_config

exec "$@"
