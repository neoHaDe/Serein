## Serein v1.2.3

Hotfix for 1.2.2. **Upgrade if you use docked windows** — 1.2.2 could freeze and crash while
dragging them.

### The crash

Detach a panel, magnet it to the main window, drag — and after a couple of seconds the app
froze and died. Windows logged it as an access violation inside `tao::…::subclass_window`
(the windowing layer under Tauri), a second one inside `memcpy`, and heap corruption in `ntdll`.

The cause was in the docking code, not in the window layer. Every OS move event ran three
backend round-trips to read the window geometry, and the handler appended that work to a promise
chain instead of replacing it. Windows emits move events dozens of times per second per window,
so a two-second drag queued hundreds of deferred jobs — each of which ended by moving a window,
which produced more move events. The queue outlived the drag, and calls left in it reached
`SetWindowPos` on a window that had already been closed.

Now a move event costs **zero** backend calls — the position comes from the event itself and the
frame padding is cached — and at most one pass is ever in flight, with everything that arrives
meanwhile collapsed into a single repeat.

### Docked windows kept losing each other

Three separate defects, all fixed:

- **Only one window could join the group.** The member list was computed once and cached
  forever; a second docked window was never picked up. The group is now rebuilt for each drag
  gesture, from a live registry the windows keep up to date themselves — so it is ready before
  the first step of a drag instead of arriving several frames late.
- **Windows detached on their own.** "The user moved me" was inferred from a 400 ms window of
  silence, so a slightly late OS event looked like a manual drag. It is now decided by comparing
  the reported position with the position we actually set, and a 6 px threshold keeps one-pixel
  jitter from unsticking anything.
- **A slow geometry query dropped a live window from the group.** Group membership was checked
  against a list that silently skips windows which fail to answer. Closed windows are now
  detected by the window list itself.

### Install

1. **Serein_1.2.3_x64-setup.exe** — NSIS installer (RU+EN).
2. **Serein_1.2.3_x64-portable.exe** — single exe. Profile in `%APPDATA%\serein` either way.

In-place upgrade from any 1.x — yes (same `dev.serein.app`).

SHA-256:

```
b33341de413a05433daea59b6e2c28dd6baa12496723f097337241b76d559789  Serein_1.2.3_x64-setup.exe
6826d40c7640f321bcb8ac4cc2596417070cecdccb3470e705e7ca2884745c11  Serein_1.2.3_x64-portable.exe
```

### Limitations

Unchanged from [1.2.2](RELEASE_NOTES_v1.2.2.md) — Windows x64 only, unsigned installer,
updater manifest not yet published.

---
Full feature list: [README](https://github.com/neoHaDe/Serein/blob/master/README.md) · Russian: [README.ru.md](https://github.com/neoHaDe/Serein/blob/master/README.ru.md)
