## Serein v1.2.2

The "replace PuTTY" release: serial consoles, SSH agent key picking, host key verification,
legacy-device compatibility, jump hosts — plus a server list you can actually organise.

Same stack: Tauri 2 + Rust + system WebView2. Installer **≈ 6 MB**.

### Serial / COM console

- **New connection type** — pick **Serial** in server settings instead of SSH.
- **Full line control** — baud rate, data bits, parity, stop bits, flow control (none / RTS-CTS / XON-XOFF).
- **DTR / RTS toggles** and **send BREAK** from the session toolbar — the two things a switch console actually needs.
- Port list shows the USB adapter's manufacturer and product name, so you can tell three FTDI dongles apart.
- Busy or unplugged ports report why: *"Порт COM5 недоступен — устройство отключено или порт занят другой программой."*

### SSH compatibility

- **Compression** — `zlib@openssh.com` / `zlib`, negotiated before `none` so it is actually used.
- **Legacy algorithms** — opt-in per server. Adds `diffie-hellman-group14-sha1`, `group1-sha1`,
  AES-CBC, 3DES-CBC and `ssh-rsa` at the **end** of the preference list: modern hosts are unaffected,
  ten-year-old switches stop refusing the handshake.
- **ProxyCommand** — connect through a jump host or any external helper, e.g.
  `ssh -W %h:%p user@jump`. Tokens `%h`, `%p`, `%r`, `%%` are expanded.

### Host keys

- **Verification dialog** on first connect and on key change, with the fingerprint and a clear
  "Отказаться" default on a changed key.
- **Known hosts management** in settings: list, remove, import from OpenSSH `known_hosts`
  (hashed `|1|` entries are skipped, not mangled).

### SSH agent

- **Key picker** — the server form lists the agent's identities with their comments, so a session
  uses the one key you meant instead of walking the whole ring and tripping `MaxAuthTries`.

### Server list

- **Groups** — collapsible, reorderable, with a settings dialog for renaming and picking members.
- **Drag and drop** — grab a server and it follows the cursor while its neighbours slide apart;
  drop it anywhere, including into another group or a collapsed one. Groups reorder the same way.
- **Right-click menus** — on the panel (new server / new group / group settings), on a server
  (connect, edit, move to group, delete) and on a group header.
- The WebView2 browser menu ("Back", "Reload", "Print", "Inspect") no longer appears anywhere.

### Fixes

- **Error text** — commands return `Result<_, String>` and Tauri throws the string itself, so the
  usual `(e as Error).message` produced `undefined`. Every dialog showed "Ошибка импорта: undefined"
  instead of the actual reason. Fixed in all 24 places.
- **Import button** — no longer runs off the panel edge, and reports a missing `~/.ssh/config` as
  a sentence rather than `undefined`.

### Install

1. **Serein_1.2.2_x64-setup.exe** — NSIS installer (RU+EN), 6.4 MB.
2. **Serein_1.2.2_x64-portable.exe** — single exe, 18.8 MB. Profile in `%APPDATA%\serein` either way.

In-place upgrade from **1.2.1**, **1.2.0** or **1.1.0** — yes (same `dev.serein.app`).

SHA-256:

```
70e310956d7b70ed24238ee025792daa2f94e077d42e3d1b057073d41a501f24  Serein_1.2.2_x64-setup.exe
43364a9475f77dbc6320c2b77c098d014f2ac2b3faad3f73b0cde692a4ee5682  Serein_1.2.2_x64-portable.exe
```

This is the first build with a **signed updater artifact** (`.sig`) — 1.2.0 and 1.2.1 shipped without one.

### Limitations

- Windows x64 only.
- The updater manifest at `nehade.xyz/updates/terminal/latest.json` still points at 1.0.0.
  The 1.2.2 manifest is generated and ready to publish, but until it is uploaded auto-update
  will not offer this version.
- Installer is unsigned (no code-signing certificate); SmartScreen will warn on first run.
- RDP and VNC are deliberately out of scope for now.

---
Full feature list: [README](https://github.com/neoHaDe/Serein/blob/master/README.md) · Russian: [README.ru.md](https://github.com/neoHaDe/Serein/blob/master/README.ru.md)
