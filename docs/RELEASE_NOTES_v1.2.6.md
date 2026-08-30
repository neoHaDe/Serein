## Serein v1.2.6

Fixes for the detached-tab window — which lost its SSH session the moment you opened Docker
or Logs — plus terminal history that survives a tab moving between windows, one command run
across several servers at once, and hardening carried over from the security audit.

### Detached tab window

- **Opening Docker / Logs / Processes no longer kills the connection.** The terminal inside a
  detached window was unmounted on every tool switch, and its teardown closed the session
  120 ms later — the window looked alive but nothing worked and returning the tab handed the
  main window a dead id. The terminal is now hidden by style and stays attached, as in the
  main window.
- **Closing the window releases the SSH session.** The close button destroys the webview
  outright, so the React cleanup never ran and the connection stayed open on the server until
  the app exited.
- **A dropped connection is visible.** The window used to report “Connected” forever; it now
  follows the session and the **Reconnect** button actually reconnects (it had no handler).

### Terminal

- **History survives a tab moving between windows.** Detaching or reattaching used to open an
  empty terminal — the screen lives inside the window, and the shell will not redraw itself.
  The last 256 KB of output is now kept per session and replayed into the new terminal, so the
  prompt and recent output are there immediately.

### Multi-host

- **Run one command on several servers** (`Ctrl+Shift+M`): pick hosts by group, confirm the
  command and the list, then watch results arrive per host with exit code, stdout and stderr.
  Hosts whose key is not in `known_hosts` — including every jump host in the chain — are
  skipped with a reason rather than trusted silently.

### Security

- Master key is wiped from memory after use; the private key file is closed to other users on
  Windows.
- Password-based backup encryption uses stronger scrypt parameters, recorded inside the packet
  so old backups still open.
- Profile and logs are readable only by their owner on Linux (`0700` / `0600`).
- Backup import warns about `ProxyCommand` entries and about key paths from the other OS.
- CI now audits dependencies on every push. `quick-xml` was updated away from two DoS
  advisories; three that cannot be closed yet are listed by ID with the reason in `ci.yml`
  (`russh` 0.60 is an SSH-layer migration, and `rsa` has no upstream fix).

### Install

1. **Serein_1.2.6_x64-setup.exe** — Windows NSIS installer (RU+EN).
2. **Serein_1.2.6_x64-portable.exe** — Windows single exe. Profile in `%APPDATA%\serein`.
3. **Serein_1.2.6_amd64.deb** — Linux package (`/usr/bin/serein`, config in `~/.config/serein`).
4. **Serein_1.2.6_amd64.AppImage** — portable Linux binary.

In-place upgrade from any 1.x on Windows and from 1.2.5 on Linux. The AppImage is large
(~100 MB) because it carries the whole WebKitGTK stack; the `.deb` uses the system one.

SHA-256:

```
0d37bb6db697c1a0bcf98ab9311a256e1d19a2e08565d4700dd97f64cb97e690  Serein_1.2.6_x64-setup.exe
8f31665532df34c55841b6bc4fe5b11611b159760b9a555dca9523bc2709a9a8  Serein_1.2.6_x64-portable.exe
ad47a161a13e6d0cc3deb4a8288621742a29065ff68ef03be926ef2d85b92c6d  Serein_1.2.6_amd64.deb
0bc5912981ed6b4d737be5e063f528983cf014be7a634220de1ef15a020110a5  Serein_1.2.6_amd64.AppImage
```

### Limitations

- `russh` 0.45 carries two known DoS advisories that need a hostile SSH server to trigger;
  fixing them means migrating to 0.60 and is the next piece of work.
- Telnet has not been run against real network hardware yet — only against the emulator
  from 1.2.4.
- Backup restore Windows→Linux does not rewrite absolute key paths.
- Drag-and-drop out of SFTP to the desktop is Windows-only.
- The Windows build is **unsigned**. SmartScreen will complain.
