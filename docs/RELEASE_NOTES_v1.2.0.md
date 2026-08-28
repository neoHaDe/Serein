## Serein v1.2.0

Docker stops being a container list and becomes a workspace: live stats, container files, Compose. Session logging finally writes files, the first run tells you where to start, and remote paths are validated before every SFTP operation.

Same stack: Tauri 2 + Rust + system WebView2. Installer **≈ 6 MB**.

### Docker

**Containers**
- Table with name, id, state, status, ports and creation time; colored state and health badge; search.
- Detail panel: live `docker stats` (CPU, memory, network, block I/O), ports, logs below.
- Files tab — browse inside a running container (`docker exec ls`).
- Shell, Logs and Restart from the detail panel; logs stream with a Stop button.

**Compose**
- Projects discovered via `docker compose ls`, with a fallback to `com.docker.compose.project` labels.
- Services with state, image and ports; up, down, start, stop, restart — per project or per service.
- Per-service logs, shell into a service, and read the compose YAML without leaving the app.

Action verbs and ids are whitelisted and quoted in Rust, not just typed in TypeScript.

### Host logs

- Highlighting for ERROR / WARN / timestamps, ANSI stripped, UTF-8 forced for `journalctl`.
- Filter, autoscroll to the newest lines, and an error report exported to `.txt`.

### Session logging

- The tab-bar toggle is no longer a stub: session output is written to `%APPDATA%\serein\logs`.
- Works for both SSH and local terminals; ANSI escapes are removed so the log stays readable.

### First run

- With no servers yet you get a welcome screen instead of an empty window: import `~/.ssh/config`, import PuTTY sessions, or add a server manually. The local terminal is one click away.

### Security and robustness

- Remote paths are validated before every SFTP operation — absolute, no `..`, no control characters.
- Secrets are never written as plain base64 anymore: without DPAPI the app refuses to store them.
- Backup password must be at least 8 characters — the `.tbk` file carries server passwords.
- Settings now reach every window, including detached SFTP, workspace and Docker log windows.
- 25 unit tests, up from 8 in 1.1.0: crypto, Docker and Compose parsing, SFTP path guard, ANSI stripping.

### Install

1. **Serein_1.2.0_x64-setup.exe** — NSIS installer (RU+EN).
2. **Serein_1.2.0_x64-portable.exe** — single exe. Profile in `%APPDATA%\serein`.

In-place upgrade from **1.1.0** — yes (same `dev.serein.app`).

### Limitations

- Windows x64 only. No SSH agent.
- The updater manifest still points at 1.0.0 — update manually.
- Installer is unsigned; SmartScreen will warn on first run.

---
Full feature list: [README](https://github.com/neoHaDe/Serein/blob/master/README.md) · Russian: [README.ru.md](https://github.com/neoHaDe/Serein/blob/master/README.ru.md)
