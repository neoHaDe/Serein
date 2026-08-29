## Serein v1.2.4

Telnet and raw TCP. With serial consoles already in 1.2.2, this closes the last thing PuTTY
still did that Serein could not.

### Telnet

- **New connection type** — pick **Telnet** in server settings. No username field: the device
  asks for credentials itself, if it asks at all.
- **Option negotiation** per RFC 854 and friends, answered the RFC 1143 way — we reply only when
  an option's state actually changes, so a polite server never drags us into an endless
  `WONT`/`DONT` exchange.
- **Terminal type** (`xterm-256color`) and **window size** (NAWS) are reported, so `top` and
  `less` on the far end stop drawing into an 80x24 box.
- **Server-side echo** is accepted rather than duplicated locally.
- **Enter sends** `CR LF` by default (the NVT end-of-line), switchable per server to `CR NUL`
  or a bare `CR` for gear that wants one of those. Wrong choice shows up as doubled blank lines
  or as a command that never runs — hence the setting.
- **BREAK / Interrupt / Are-You-There** buttons in the status bar. On network gear this is the
  only way to stop a hung `ping` once the device has stopped reading the data stream.

### Raw TCP

Same transport with nothing done to the bytes: no negotiation, no line-ending rewriting.
For console servers — Cisco and Digi listen on port 2000+ per line — and for poking a text
protocol by hand.

### How this was tested without hardware

No spare Ethernet cable, no switch on the bench. So `scripts/telnetd.js` impersonates one:
it pushes `WILL ECHO` / `WILL SGA`, demands `DO TTYPE` / `DO NAWS` plus an unsupported
`DO TSPEED`, and logs everything it receives. Two tests talk to it over a real socket and skip
themselves when it is not running. Its log confirmed the terminal type, the window size, a
single refusal of the unsupported option, and `LF` following `CR` after Enter. 15 tests in the
module, 64 across the crate.

### Fix

- **Pane type was hardcoded to SSH** when splitting a pane and when restoring sessions after a
  restart. A serial profile opened there as SSH and failed immediately — present since serial
  landed in 1.2.2. The pane type now comes from the profile everywhere.

### Install

1. **Serein_1.2.4_x64-setup.exe** — NSIS installer (RU+EN).
2. **Serein_1.2.4_x64-portable.exe** — single exe. Profile in `%APPDATA%\serein` either way.

In-place upgrade from any 1.x — yes (same `dev.serein.app`).

SHA-256:

```
e7ed193eb108c0b0e729f3e4bce05c8f53b0a6acde2d2dbc593ebe19eb272448  Serein_1.2.4_x64-setup.exe
bd8456c736a066df146737cbdd0f191746c91dd7fb7cedfbda571788dc41edbc  Serein_1.2.4_x64-portable.exe
```

### Limitations

- Telnet has not been run against real network hardware yet — only against the emulator above.
- Telnet is **not encrypted**. Do not use it across a network you do not control.
- Windows x64 only, unsigned installer, updater manifest not yet published.

---
Full feature list: [README](https://github.com/neoHaDe/Serein/blob/master/README.md) · Russian: [README.ru.md](https://github.com/neoHaDe/Serein/blob/master/README.ru.md)
