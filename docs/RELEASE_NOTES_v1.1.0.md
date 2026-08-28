## Serein v1.1.0

Крупный выпуск после **1.0.0**: быстрый SFTP с очередью передач, Server Workspace у SSH-вкладки, drag-and-drop с Проводником, открепление вкладок и панелей в отдельные окна.

Тот же стек: Tauri 2 + Rust + системный WebView2. Установщик **≈ 6 МБ**.

### SFTP

**Скорость и очередь**
- Параллельные передачи: пул 4 канала по умолчанию, до 8 — в «Настройки».
- Менеджер передач: скорость, ETA, прогресс, пауза, повтор, пропуск дублей.
- Download: pipeline READ (до 32 в полёте) и чанк до 256 КиБ — на гигабите LAN ~112 МБ/с вместо ~8 МБ/с.
- Отмена ✕ сразу рвёт канал, без зависшей строки «активна».

**Проводник**
- Колонки как в Explorer: имя, тип, права, размер, дата; ресайз, сортировка, Ctrl/Shift-выделение.
- Drag-and-drop с Проводником; chmod, скрытые файлы, symlink follow, превью картинок.

### Server Workspace

- Рельса: Terminal, Docker, Logs, Processes (+ метрики), Services, Tunnels.
- SFTP — боковая панель из TabBar.

### Окна и открепление

- ↗ вкладку или панель workspace; ← вернуть в main (SSH-сессия не рвётся).
- Aux сворачиваются независимо; свёрнутые — клик по Serein или кнопка в статус-баре.

### Терминал

- Ctrl+Shift+C/V, батчинг вывода, Win32 clipboard.

### Установка

1. **Serein_1.1.0_x64-setup.exe** — установщик NSIS (RU+EN).
2. **Serein_1.1.0_x64-portable.exe** — один exe. Профиль в %APPDATA%\serein.

In-place с **1.0.0** — да.

### SHA-256

- setup: CE0C1632B885841B7991DFBCB5B71989D538CBD0E8FA507F721C5BB91F6FE81A
- portable: CAE1CFDBE1618F7AFC7D4C38906BBF24BFA047AD88A8A6C247E07D051D1BAE35

### Ограничения

- Windows x64. Нет SSH-агента. Updater под 1.1.0 не настраивали.

---
[README](https://github.com/neoHaDe/Serein/blob/master/README.md) · [README.en.md](https://github.com/neoHaDe/Serein/blob/master/README.en.md)