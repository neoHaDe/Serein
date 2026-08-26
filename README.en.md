<div align="center">

<img src="src-tauri/icons/128x128.png" width="96" alt="Serein icon" />

# Serein

**Desktop SSH / SFTP client — everything in one window.**

Tabs and split panes, an SFTP file manager with an editor,
port forwards, resource monitoring, a Docker panel, and a local terminal —
in an installer of about **3 MB**.

Free, open source, MIT. Windows x64, **v0.1.0**.

[![Tauri](https://img.shields.io/badge/Tauri-2-24C8DB?logo=tauri&logoColor=white)](https://tauri.app)
[![Rust](https://img.shields.io/badge/Rust-stable-000000?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![React](https://img.shields.io/badge/React-18-61DAFB?logo=react&logoColor=black)](https://react.dev)
[![TypeScript](https://img.shields.io/badge/TypeScript-5-3178C6?logo=typescript&logoColor=white)](https://www.typescriptlang.org)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](#license)

Windows · no bundled Chromium (system WebView2) ·
[Русская версия](README.md)

### → [Download the latest release](../../releases/latest)

<img src="docs/screenshot.png" width="820" alt="Serein screenshot" />

</div>

---

## Why Serein

The UI runs in the system WebView. SSH, SFTP, crypto, and the PTY live in one Rust binary.
No Chromium tax. We are not racing Tabby on feature count. The point is a **server workspace**
(terminal, files, Docker, logs, resources, tunnels), not “yet another SSH client”.

| | **Serein (Tauri)** | Typical Electron client |
| --- | :---: | :---: |
| Installer size | **≈ 3 MB** | ≈ 85 MB |
| Idle RAM | **≈ 33 MB** | 150–250 MB |
| SSH engine | pure Rust [`russh`](https://github.com/Eugeny/russh) | libssh2 / native |
| Runtime | system WebView2 | full Chromium |

Numbers come from a live `tauri dev` session and an NSIS-size estimate on Windows.
A weak laptop will not magically match 33 MB.

---

## Features

### Terminal and UX
- **Multiple SSH tabs** and **split panes** (tree with drag-resize, pick a server per pane)
- **Local terminal** with a sensible shell (PowerShell → cmd, or a custom / WSL shell)
- Search (`Ctrl+F`), zoom, **17 UI themes**, **compact mode**
- **Broadcast input** (current tab only), tab restore, tab drag
- Keyboard pane navigation, remappable shortcuts, **command palette** (`Ctrl+Shift+P`)

### Connections
- Sidebar with groups and search, **live connection status**
- Auth: **password · key · keyboard-interactive 2FA**
- **ProxyJump / bastion** chains (recursive, `direct-tcpip`)
- **TOFU known-hosts** on every hop
- Import from **`~/.ssh/config`** and **PuTTY** sessions

### Files (SFTP)
- Browse with **clickable breadcrumbs**, **inline rename**, drag & drop into the window
- **Recursive transfers** with progress, dual-pane (local ↔ remote)
- **Built-in editor** (CodeMirror 6) — atomic save back to the server
- **External editor** — OS default app, re-upload on save

### Tunnels and ops
- Forwards: **local `-L`**, **remote `-R`**, **dynamic SOCKS5 `-D`**
- **Resource monitor** — CPU / RAM / disk / load (`/proc` + `df`)
- **Docker panel** — list, start/stop/restart/remove, logs, shell into a container

### Security and storage
- Secrets via **DPAPI** plus an optional **master password**
  (scrypt → AES-256-GCM); the UI never gets passwords or keys
- Encrypted **`.tbk` backup** of servers, settings, and snippets
- **SSH keygen** (ed25519 / RSA) + `ssh-copy-id`

---

## Quick start

1. Install `Serein_0.1.0_x64-setup.exe` from [Releases](../../releases/latest).
2. Import `~/.ssh/config` or add a host by hand.
3. Connect. The local terminal works with no SSH at all.

Target: install → first session in under two minutes.

---

## Requirements

| Need | Answer |
| --- | --- |
| OS | Windows 10/11, x64. No macOS or Linux builds yet |
| WebView2 | already on current Windows; nothing extra to install |
| Privileges | admin is not required for daily use |
| SSH agent | **not yet** — password, key file, or keyboard-interactive |

---

## Install

Download **`Serein_0.1.0_x64-setup.exe`** from
[Releases](../../releases/latest) and run it.

The build is **unsigned**. SmartScreen will complain. *More info → Run anyway*.
If the release notes publish a SHA-256, check that instead of clicking through blindly.

The updater endpoint is wired (`nehade.xyz/updates/terminal/`). Do not rely on it
while the installer is unsigned.

---

## Stack

| Layer | Tech |
| --- | --- |
| Shell | **[Tauri 2](https://tauri.app)** (Rust, system WebView2) |
| Frontend | **React 18** · **TypeScript 5** · **Vite** |
| Terminal | [`@xterm/xterm`](https://xtermjs.org) |
| Editor | [CodeMirror 6](https://codemirror.net) |
| SSH / SFTP | [`russh`](https://github.com/Eugeny/russh) · [`russh-sftp`](https://github.com/AspectUnk/russh-sftp) |
| Local PTY | [`portable-pty`](https://crates.io/crates/portable-pty) |
| Crypto | `aes-gcm` · `scrypt` · `ssh-key` · Windows DPAPI |

---

## Architecture

```
┌─────────────────────────── WebView (React) ───────────────────────────┐
│  App · TabBar · Sidebar · SftpPanel · Monitor · Docker · CodeEditor    │
│  └── src/api  ──  window.api bridge  (invoke / listen)                 │
└───────────────────────────────┬───────────────────────────────────────┘
                     Tauri commands and events
┌───────────────────────────────┴───────────────────────────────────────┐
│  Rust backend (src-tauri/src)                                          │
│  ssh · sftp · tunnels · monitor · docker · pty · store · vault ·       │
│  crypto · dpapi · keygen · importers · knownhosts · remoteedit         │
└────────────────────────────────────────────────────────────────────────┘
```

- React talks to Rust through a thin `window.api` bridge (`invoke` / `listen`).
- One SSH connection multiplexes **shell + SFTP + exec + tunnels**. The handle is locked
  only briefly, so opening channels does not stall the others.
- Secrets decrypt **only in Rust**, at connect time. Outside Windows there is no DPAPI:
  a `plain:` fallback exists (this build is Windows).

---

## Build from source

You need [Rust](https://rustup.rs) (stable) and [Node.js](https://nodejs.org) 18+.
In non-interactive PowerShell, `cargo` is often off PATH — prepend `$env:USERPROFILE\.cargo\bin`.

The dev server binds **`127.0.0.1:1420`**, not `localhost` (IPv4 vs IPv6 otherwise sits on
“Waiting for frontend” forever).

```bash
npm install
npm run tauri dev
npm run tauri build
```

Installer output: `src-tauri/target/release/bundle/nsis/`.

```bash
npm run typecheck
cargo check --manifest-path src-tauri/Cargo.toml
```

---

## Data and diagnostics

Profiles, secrets, known_hosts, vault live in `%APPDATA%\serein\`
(`servers.json`, `secrets.json`, `vault.json`, `known_hosts.json`, …).

Secrets never go to the UI. There is no dedicated log file yet: use the app window
and `npm run tauri dev` output.

---

## Known limitations

- **Windows x64** only. No `.dmg` / `.AppImage`.
- Installer is **unsigned**. SmartScreen will fight you.
- No **SSH agent** and no **agent forwarding**.
- No drag-out of files onto the desktop.
- No proper transfer-manager UI (pause / cancel / retry).
- Remote `-R`, TOFU, the external editor, SFTP, and the master-password vault exist in code;
  not every path has been live-tested in production. File a bug, do not assume silence is OK.
- Not on the near-term list: cloud sync, mobile, plugins, Telnet/RDP/VNC, a generic LLM chat pane.

Product plan: a reliable SSH core, then Server Workspace, then Copilot. Do not skip the first layer.

---

## License

[MIT](LICENSE) © 2026 HaDe
