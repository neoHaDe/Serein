<div align="center">

<img src="src-tauri/icons/128x128.png" width="96" alt="Иконка TermiNAL" />

# TermiNAL

**Десктоп-клиент SSH / SFTP «всё в одном окне».**

Вкладки и сплит-панели, файловый менеджер SFTP со встроенным редактором,
проброс портов, мониторинг ресурсов, панель Docker и локальный терминал —
в установщике на **≈ 3 МБ**.

Бесплатно, открытый код, MIT. Windows x64, **v0.1.0**.

[![Tauri](https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white)](https://tauri.app)
[![Rust](https://img.shields.io/badge/Rust-stable-000000?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![React](https://img.shields.io/badge/React-18-61DAFB?logo=react&logoColor=black)](https://react.dev)
[![TypeScript](https://img.shields.io/badge/TypeScript-5-3178C6?logo=typescript&logoColor=white)](https://www.typescriptlang.org)
[![Лицензия: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](#-лицензия)

Windows · без своего Chromium (системный WebView2) ·
[English version](README.en.md)

### → [Скачать последний выпуск](../../releases/latest)

<img src="docs/screenshot.png" width="820" alt="Скриншот TermiNAL" />

</div>

---

## ✨ Почему TermiNAL

Интерфейс в системном WebView, SSH / SFTP / шифрование / PTY — в одном Rust-бинарнике.
Не тащим Chromium. Не пытаемся обогнать Tabby списком галочек: цель — **рабочее место
по серверу** (терминал, файлы, Docker, логи, ресурсы, туннели), а не «ещё один SSH-клиент».

| | **TermiNAL (Tauri)** | Типичный Electron-клиент |
| --- | :---: | :---: |
| Размер установщика | **≈ 3 МБ** | ≈ 85 МБ |
| Память в простое | **≈ 33 МБ** | 150–250 МБ |
| SSH-движок | чистый Rust [`russh`](https://github.com/Eugeny/russh) | libssh2 / нативный |
| Рантайм | системный WebView2 | полный Chromium |

Цифры — живой `tauri dev` / оценка NSIS на Windows. На слабом ноутбуке те же 33 МБ не обещаем.

---

## 🚀 Возможности

### Терминал и UX
- **Несколько SSH-вкладок** + **сплит-панели** (дерево с перетаскиванием границ, выбор сервера на панель)
- **Локальный терминал** с умным выбором shell (PowerShell → cmd, либо свой / WSL)
- Поиск (`Ctrl+F`), зум, **17 тем на весь интерфейс**, **компактный режим**
- **Broadcast-ввод** (в пределах текущей вкладки), восстановление вкладок, перетаскивание вкладок
- Навигация по панелям с клавиатуры, настраиваемые хоткеи, **командная палитра** (`Ctrl+Shift+P`)

### Подключения
- Сайдбар с группами и поиском, **живой статус подключения**
- Аутентификация: **пароль · ключ · keyboard-interactive 2FA**
- **ProxyJump / бастион** — цепочки (рекурсивно, через `direct-tcpip`)
- **TOFU-проверка known-hosts** на каждом хопе
- Импорт из **`~/.ssh/config`** и сессий **PuTTY**

### Файлы (SFTP)
- Просмотр с **кликабельными хлебными крошками**, **инлайн-переименование**, drag & drop внутрь окна
- **Рекурсивные передачи** с прогрессом, двухпанельный режим (локально ↔ сервер)
- **Встроенный редактор** (CodeMirror 6) — атомарное сохранение прямо на сервер
- **Внешний редактор** — открывает файл в редакторе ОС и сам заливает при сохранении

### Туннели и эксплуатация
- Проброс портов: **локальный `-L`**, **обратный `-R`**, **динамический SOCKS5 `-D`**
- **Мониторинг ресурсов** — CPU / RAM / диск / load (опрос `/proc` + `df`)
- **Панель Docker** — список, старт/стоп/рестарт/удаление, логи, shell в контейнер

### Безопасность и хранение
- Секреты шифруются через **DPAPI** + опциональный **мастер-пароль**
  (scrypt → AES-256-GCM); в UI пароли и ключи не отдаются
- Зашифрованный **бэкап `.tbk`** серверов, настроек и сниппетов
- **Генерация SSH-ключей** (ed25519 / RSA) + `ssh-copy-id`

---

## Быстрый старт

1. Поставь `TermiNAL_0.1.0_x64-setup.exe` из [Releases](../../releases/latest).
2. Импортируй `~/.ssh/config` или добавь сервер вручную.
3. Подключись. Локальный терминал работает и без SSH.

Цель: установка → первое соединение меньше чем за две минуты.

---

## Что нужно

| Требование | Ответ |
| --- | --- |
| Система | Windows 10/11, x64. macOS и Linux пока не собираются |
| WebView2 | уже есть в актуальном Windows; отдельно ставить не нужно |
| Права | администратор для повседневной работы не нужен |
| SSH-агент | **пока нет** — пароль, файл ключа или keyboard-interactive |

---

## 📦 Установка

Скачай **`TermiNAL_0.1.0_x64-setup.exe`** со страницы
[Releases](../../releases/latest) и запусти.

Сборка **не подписана** — SmartScreen ругнётся. *Подробнее → Выполнить в любом случае*.
Проверяй SHA-256 из описания выпуска, если он там есть.

Автообновление в конфиге заведено (`nehade.xyz/updates/terminal/`), на неподписанном
установщике на него не рассчитывай.

---

## 🛠 Стек

| Слой | Технологии |
| --- | --- |
| Оболочка | **[Tauri 2](https://tauri.app)** (Rust, системный WebView2) |
| Фронтенд | **React 18** · **TypeScript 5** · **Vite** |
| Терминал | [`@xterm/xterm`](https://xtermjs.org) |
| Редактор | [CodeMirror 6](https://codemirror.net) |
| SSH / SFTP | [`russh`](https://github.com/Eugeny/russh) · [`russh-sftp`](https://github.com/AspectUnk/russh-sftp) |
| Локальный PTY | [`portable-pty`](https://crates.io/crates/portable-pty) |
| Криптография | `aes-gcm` · `scrypt` · `ssh-key` · Windows DPAPI |

---

## 🏗 Архитектура

```
┌─────────────────────────── WebView (React) ───────────────────────────┐
│  App · TabBar · Sidebar · SftpPanel · Monitor · Docker · CodeEditor    │
│  └── src/api  ──  мост window.api  (invoke / listen)                   │
└───────────────────────────────┬───────────────────────────────────────┘
                     команды и события Tauri
┌───────────────────────────────┴───────────────────────────────────────┐
│  Rust-бэкенд (src-tauri/src)                                           │
│  ssh · sftp · tunnels · monitor · docker · pty · store · vault ·       │
│  crypto · dpapi · keygen · importers · knownhosts · remoteedit         │
└────────────────────────────────────────────────────────────────────────┘
```

- React говорит с Rust через тонкий мост `window.api` (те же имена, что у Electron-ветки).
- Одно SSH-соединение мультиплексирует **shell + SFTP + exec + туннели**; handle берётся
  коротким async-локом, каналы не ждут друг друга.
- Секреты расшифровываются **только в Rust**, в момент подключения. Вне Windows DPAPI нет:
  там запасной `plain:` (эта сборка — Windows).

---

## 👩‍💻 Сборка из исходников

Нужны [Rust](https://rustup.rs) (stable) и [Node.js](https://nodejs.org) 18+.
В неинтерактивном PowerShell `cargo` часто не в PATH — добавь `$env:USERPROFILE\.cargo\bin`.

Dev-сервер слушает **`127.0.0.1:1420`**, не `localhost` (IPv4/IPv6 иначе вечно «Waiting for frontend»).

```bash
npm install
npm run tauri dev
npm run tauri build
```

Установщик: `src-tauri/target/release/bundle/nsis/`.

```bash
npm run typecheck
cargo check --manifest-path src-tauri/Cargo.toml
```

---

## Данные и диагностика

Профили, секреты, known_hosts, vault — в `%APPDATA%\term-tauri\`
(`servers.json`, `secrets.json`, `vault.json`, `known_hosts.json`, …).

Секреты в UI-слой не едут. Логи отдельным файлом пока не пишем: смотри окно приложения
и вывод `npm run tauri dev`.

---

## Известные ограничения

- Только **Windows x64**. `.dmg` / `.AppImage` нет.
- Установщик **без цифровой подписи**. SmartScreen будет спорить.
- Нет **SSH-агента** и **agent-forwarding**.
- Нет drag-out файла на рабочий стол.
- Нет нормального UI очереди передач: пауза / отмена / повтор как в «менеджере загрузок».
- Часть фич (обратный туннель `-R`, TOFU, внешний редактор, SFTP, мастер-пароль) в коде есть;
  не всё гонялось вживую на проде. Баг — issue, не молчание.
- Не цель ближайших релизов: облачный sync, мобилка, плагины, Telnet/RDP/VNC, «просто чат с LLM».

Дорожная карта продукта: Server Workspace, потом Copilot. Слой «надёжный SSH» — сначала.

---

## 📄 Лицензия

[MIT](LICENSE) © 2026 HaDe
