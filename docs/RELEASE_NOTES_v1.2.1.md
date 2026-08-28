## Serein v1.2.1

Patch release: SSH Agent finally works, detached tabs keep SFTP beside the terminal, and the Processes tab shows server load at a glance.

Same stack: Tauri 2 + Rust + system WebView2. Installer **≈ 6 MB**.

### SSH Agent

- **Authentication** — choose **SSH agent** in server settings; keys from the local agent are tried automatically (Windows: `OpenSSH Authentication Agent` + `ssh-add`; pipe `\\.\pipe\openssh-ssh-agent`).
- **Agent forwarding** — optional checkbox to expose your local agent on the remote host (`auth-agent-req@openssh.com`).

### UI fixes

- **Detached tab window** — SFTP stays in a side column next to the terminal (CSS grid + cache-bust on aux webview recreate).
- **Processes dashboard** — ring gauges for CPU/RAM/disk, load average with severity tags, mini CPU/MEM bars in the process table.

### Install

1. **Serein_1.2.1_x64-setup.exe** — NSIS installer (RU+EN).
2. **Serein_1.2.1_x64-portable.exe** — single exe. Profile in `%APPDATA%\serein`.

In-place upgrade from **1.2.0** or **1.1.0** — yes (same `dev.serein.app`).

### Limitations

- Windows x64 only.
- The updater manifest still points at 1.0.0 — update manually.
- Installer is unsigned; SmartScreen will warn on first run.

---
Full feature list: [README](https://github.com/neoHaDe/Serein/blob/master/README.md) · Russian: [README.ru.md](https://github.com/neoHaDe/Serein/blob/master/README.ru.md)