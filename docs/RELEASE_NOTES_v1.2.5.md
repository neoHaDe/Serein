## Serein v1.2.5

First **Linux** release (`.deb` + AppImage) alongside Windows, plus Linux windowing fixes,
a keyring leak fix, and **GitHub Dark** as the default theme.

### Linux

- **Installers** — `Serein_1.2.5_amd64.deb` and `Serein_1.2.5_amd64.AppImage`.
- **Secrets** via Secret Service (`keyring`), not DPAPI.
- **Window group move** — docked windows follow the main one (`windows_nudge_group`); a late
  geometry reply no longer rebuilds offsets mid-drag and pulls the group apart.
- **Raise / restore** for the “one taskbar button” mode works on Linux (were stubs before).
- **Settings → Profile path** — shows the real config directory, so a wrong `HOME` from the
  app menu is obvious.
- **Keyring cleanup** — overwriting or deleting a password removes the old Secret Service
  entry instead of leaving orphans forever.
- Build tooling: `libudev-dev` required, safer VM bootstrap (no baked-in SSH key, no wipe of
  an existing clone, `apt update` may fail without aborting).

### Theme

- Default UI / terminal theme is **GitHub Dark** (new profiles and first launch). Existing
  saved themes are unchanged.

### Windows

Same binary line as 1.2.4 plus the theme default and the window-group rebuild fix (the late
geometry reply could affect Windows too). NSIS + portable as before.

**Hotfix (2026-08-30):** re-uploaded Windows installers — docked aux windows no longer crash
the app when the main window is dragged for a long time (regression from the Linux
`windows_nudge_group` path; Windows again uses emit + self-move only).

### Install

1. **Serein_1.2.5_x64-setup.exe** — Windows NSIS installer (RU+EN).
2. **Serein_1.2.5_x64-portable.exe** — Windows single exe. Profile in `%APPDATA%\serein`.
3. **Serein_1.2.5_amd64.deb** — Linux package (`/usr/bin/serein`, config in `~/.config/serein`).
4. **Serein_1.2.5_amd64.AppImage** — portable Linux binary.

In-place upgrade from any 1.x on Windows — yes (same `dev.serein.app`).
Linux is a new platform line; restore a Windows `.tbk` if you need the profile, then fix
any `C:\…` key paths by hand (path normalisation is still TODO).

SHA-256:

```
c569733c1e16490b727be10bc83ccf21f0d80241cfe87a3c1baf9f9241fa9ecd  Serein_1.2.5_x64-setup.exe
cfd5bb91b8754e825772be7abb1b1048d50fe009de527cfd561ab249e9fd3f0d  Serein_1.2.5_x64-portable.exe
73ff9f5becde785ae9c8939b085246e21d309721cacc3a3f922028b4f8c68f33  Serein_1.2.5_amd64.deb
1cb2eaab71c212af319c2c64b4c21cd59e3141357340b62b0582732597241b7c  Serein_1.2.5_amd64.AppImage
```

### Limitations

- Telnet has not been run against real network hardware yet — only against the emulator
  from 1.2.4.
- Backup restore Windows→Linux does not rewrite absolute key paths.
- Launching from some Linux app menus may still open with a collapsed / empty-looking
  sidebar; check the profile path in Settings if servers seem missing.
- The Windows build is **unsigned**. SmartScreen will complain.
