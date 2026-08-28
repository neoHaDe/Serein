## Serein v1.1.0

Major release after **1.0.0**: fast SFTP with a transfer queue, Server Workspace on SSH tabs, drag-and-drop with Windows Explorer, detach/reattach for tabs and workspace panels.

Same stack: Tauri 2 + Rust + system WebView2. Installer **≈ 6 MB**.

### SFTP

**Speed and queue**
- Parallel transfers: pool of 4 channels by default, up to 8 in Settings.
- Transfer manager: speed, ETA, progress, pause, retry, skip duplicates.
- Download: READ pipeline (up to 32 in flight) and chunks up to 256 KiB — ~112 MB/s on gigabit LAN vs ~8 MB/s before.
- Cancel (✕) tears down the channel immediately — no stuck "active" row.

**File browser**
- Explorer-like columns (name, type, mode, size, date); sort, resize, Ctrl/Shift selection.
- Drag-and-drop with Windows Explorer; chmod, hidden files, symlink follow, image preview.

### Server Workspace

- Rail on SSH tabs: Terminal, Docker, Logs, Processes (+ metrics), Services, Tunnels.
- SFTP stays a side panel from TabBar.

### Windows and detach

- Detach an SSH/local tab or any workspace panel; reattach with ← (SSH session stays open).
- Aux windows minimize independently; restore via Serein on the taskbar or the main status bar button.
- Window magnet and saved geometry for detached panels.

### Terminal

- Ctrl+Shift+C/V, Win32 clipboard, SSH output batching.

### Install

1. **Serein_1.1.0_x64-setup.exe** — NSIS installer (RU+EN).
2. **Serein_1.1.0_x64-portable.exe** — single exe. Profile in `%APPDATA%\serein`.

In-place upgrade from **1.0.0** — yes (same `dev.serein.app`).

### SHA-256

- setup: CE0C1632B885841B7991DFBCB5B71989D538CBD0E8FA507F721C5BB91F6FE81A
- portable: CAE1CFDBE1618F7AFC7D4C38906BBF24BFA047AD88A8A6C247E07D051D1BAE35

### Limitations

- Windows x64 only. No SSH agent. Updater not wired for 1.1.0 yet.

---
Full feature list: [README](https://github.com/neoHaDe/Serein/blob/master/README.md) · Russian: [README.ru.md](https://github.com/neoHaDe/Serein/blob/master/README.ru.md)