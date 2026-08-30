# Serein — Linux port

Branch `linux/port` carried the work; **1.2.5** ships the first public Linux installers
(`.deb` + AppImage) alongside Windows.

## Decisions

- Formats: **AppImage + .deb**
- Secrets: **keyring** crate (libsecret / Secret Service), not raw libsecret
- Build and validation: **Ubuntu / Astra VM on Hyper-V**
- Main `tauri.conf.json` keeps NSIS-only targets; Linux bundling lives in
  `tauri.linux.conf.json` and is merged in by `scripts/build-linux.sh`
- First Linux release: **1.2.5** (earlier plan said 1.3.0 — superseded once the VM path worked)

## Build host setup

```bash
sudo apt update || true
sudo apt install -y build-essential curl git pkg-config libssl-dev \
  libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev patchelf \
  libudev-dev
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source "$HOME/.cargo/env"
# Node 22 from NodeSource — distro Node 18 leaves npm broken on Astra
curl -fsSL https://deb.nodesource.com/setup_22.x | sudo -E bash -
sudo apt install -y nodejs && sudo apt-mark hold nodejs || true
git clone https://github.com/neoHaDe/Serein.git && cd Serein
git checkout linux/port   # or master once 1.2.5 is merged
npm ci && npm run smoke:linux
./scripts/build-linux.sh
```

Two traps worth knowing before you spend an hour on them:

- **`libudev-dev` is not optional.** The `serialport` crate pulls in `libudev-sys`, whose
  build script calls `pkg-config libudev` and panics without it. Without this package both
  the smoke run and the bundle fail, and the error names `libudev-sys`, not Serein.
- **Do not use the distro Node on Astra.** Node 18 from the archive leaves `npm` broken.
  Install Node 22 from NodeSource and `apt-mark hold nodejs` so an upgrade does not undo it.
- `apt update` on Astra can exit non-zero on a repo it cannot reach. Do not treat that as
  a hard failure in bootstrap scripts.

`scripts/bootstrap-linux-vm.sh` does all of the above on a fresh VM.

## npm scripts

- `npm run smoke:linux` — tsc + `cargo check` + `cargo test`
- `npm run build:linux` — `.deb` + AppImage into `src-tauri/target/release/bundle/`

## Bundling

`tauri.linux.conf.json` sets `category: "DeveloperTool"`. It must not be `Network`: the bundler
validates the value against the freedesktop list and aborts with `invalid category` — the
`.desktop` spec has no `Network` main category.

Artifacts, for scale: `.deb` ~11 MB, AppImage ~101 MB (it carries its own runtime).
The `.deb` installs the binary to `/usr/bin/serein`.

The `TAURI_SIGNING_PRIVATE_KEY` warning during a Linux build is expected — the updater
signature is produced on the Windows release host, and it does not block bundling.

## Secrets

- Linux keeps the payload in the **keyring**; `servers.json` only holds a `kr:{uuid}` handle.
- **No Secret Service running** (headless, minimal desktop) means secrets are only stored
  when a master password is set — the master-encrypted `mk:` blob is safe to write to disk
  on its own. Without either, nothing is saved, exactly as on Windows without DPAPI.
- **Windows DPAPI secrets do not migrate.** A backup restored from Windows brings the
  profiles across but not the passwords, and any `privateKeyPath` still points at `C:\…`.
  Path normalisation on restore is a separate task — see Migration in the roadmap.

## Config location

`~/.config/serein/` — `servers.json`, `settings.json`, `known_hosts.json`, and the rest,
mirroring `%APPDATA%\serein` on Windows.

## Known gaps

- Drag-and-drop of files **out of** SFTP to the desktop is Windows-only (the `drag` crate).
  Everything else in SFTP works.
- PuTTY session import reads the Windows registry; the button is hidden on Linux.
- Launching from the application menu has shown a different UI than launching from a
  terminal. Suspected environment difference rather than a second binary — confirm the
  sidebar state first, then adjust the `.desktop` entry.
