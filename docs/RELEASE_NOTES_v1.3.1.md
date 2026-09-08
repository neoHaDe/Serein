## Serein v1.3.1

The remote desktop over RDP, and the fix for a defect that had been quietly slowing every call
between the interface and the core since the beginning.

### Remote desktop over RDP

The slot promised in 1.3.0 is filled. Like VNC, it goes through a channel inside the SSH session
that is already open — nothing new is exposed to the network, and the server's `3389` may keep
listening on loopback only.

- **The protocol is parsed by a separate program**, `serein-rdp`, shipped next to the
  application. Not an architectural preference: IronRDP, through `picky`, pins sixteen RustCrypto
  crates to release candidates, and the SSH core uses the same sixteen at stable versions. One
  dependency tree cannot hold both — verified by patching and by downgrading `russh` to the same
  line; behind every pin removed stood the next one. The isolation pays for itself: a decoder for
  someone else's protocol cannot take the application down with it.
- **One login, not two.** Credentials from our form are sent in the client info PDU with the
  autologon flag, so the server logs the user in directly instead of showing its own login window
  on top of the picture. The flag can be turned off when the server should ask for itself.
- **Connection settings**: resolution, colour depth (32/24/16 bit), traffic saving (no wallpaper,
  themes, menu animation), and the autologon switch above.
- **Resolution follows the window.** Stretching the window makes the server redraw the desktop at
  the new size over the Display Control channel, with no reconnection — half a second after the
  edge is released, because every such request costs the server a full reactivation sequence.
  Servers without that channel keep the picture fitted to the window, as before.
- **xrdp is installed and started from a button**, the way VNC already was. On Debian and Fedora
  `xorgxrdp` is installed alongside it — without it the login ends in a blank grey screen instead
  of a desktop. On Arch the install is refused with the reason: the package lives only in the AUR.
  The service is enabled, not merely started, so the desktop survives a server reboot.
- The sudo password goes to standard input, never into the command line, which is visible in the
  server's process list to everyone with an account on the box.

### The application was not allowed to use its own IPC

The content security policy carried no `connect-src`, so `default-src 'self'` applied and the
internal Tauri address (`http://ipc.localhost`) did not qualify. Every call from the interface to
the core was refused and silently fell back to `postMessage`.

Visible consequence: desktop frames larger than a kilobyte are delivered through `fetch`, which
was blocked, and the fallback path delivered something other than an `ArrayBuffer` — parsing threw
and the frame was dropped. On RDP that showed as a black rectangle with a correct size in the
header. VNC lost its larger tiles the same way, which had been read as "the picture updates oddly".

The policy now names the IPC origin explicitly. Frame parsing also survives a non-`ArrayBuffer`
payload instead of throwing: a silent black screen is the worst possible failure, and one class of
it is now impossible.

### The session belongs to the SSH connection, not to a window

- **Switching tabs no longer drops the desktop.** The panel is hidden, not unmounted — the same
  rule the terminal has followed since 1.2.6. Going to Docker and back used to mean connecting
  again, password included.
- **Detaching moves the live session into the new window.** The frame receiver can now be
  swapped while the session runs, and a panel opening in the second window asks the core whether a
  desktop is already open before offering a login form. The first frame after the move is a full
  repaint: a motionless desktop sends nothing for hours on its own.
- Detaching a desktop panel used to open a hard-coded VNC panel regardless of what was detached.
  It now opens the chooser, which picks up whatever is live.
- The session closes on an explicit action — the back button in the panel header — and together
  with the SSH session itself.

### Keyboard

Holding a modifier made the browser repeat the key-down event, and the RDP input library turns
every repeat into a release followed by a fresh press. For letters that is correct; in the middle
of Alt+Shift it reads as a second layout switch, so the layout changed and immediately changed
back. Modifier auto-repeat is no longer sent at all.

The other half of the same defect: Alt+Shift also switches the local Windows layout, and the
window often loses the `keyup` while that happens, leaving the server with a modifier held down.
Everything held is now released when the canvas loses focus. Both fixes apply to VNC as well.

### Interface wording

Captions across the application now say what a thing is, not why it was built that way. The
reasoning stayed where it is useful — in the code. Two paragraphs about the VNC password protocol
became one sentence; the note under the sudo field became "the password is not saved"; the tiles
of the desktop chooser say "connection inside the SSH channel, login with the server account" and
"login through the SSH channel, no port forward required".

### Build

CI now builds the RDP helper before compiling the application: Tauri validates the presence of
the external binary at build time, and without the file even `cargo test` refused to compile. A
side benefit is that the helper is now built on Linux and Windows in a clean environment on every
push, not only on the developer's machine.

### Checksums

```
```
