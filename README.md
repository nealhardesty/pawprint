# PAWPRINT

**Know which machine you're on.** PAWPRINT puts a tiny pixel-art animal in a
shipping crate in the corner of your screen, stamped with the machine's name.
Remote into `moose` and there's a moose. Switch to `owl` and an owl slowly
turns its head to look at you.

```
      ,_,                     ____
     (O,O)                  ^ /    \ ^          (=^.^=)
     (   )                  ( o  o )            (  w  )
  ┌──"─"──┐              ┌───\    /───┐       ┌────────┐
  │  OWL  │              │   MOOSE    │       │ BUILD7 │
  └───────┘              └────────────┘       └────────┘
```
*(Real art is 32×32 pixel sprites. This is the idea in ASCII.)*

- **Always on top, never in the way.** Clicks go straight through it, it never
  steals focus, and it stays out of the taskbar, Dock, and Alt-Tab.
- **Works through any remote desktop.** RDP, VNC, Jump Desktop, Parsec,
  Apollo/Moonlight. It runs on the machine you're looking at, so whatever you
  connect with shows it.
- **Picks the right animal automatically.** A host named `owl`, `owl-02`, or
  `prod-owl.lab.example` gets the owl. Names with no animal in them still get
  their own consistent mascot.
- **100 animated mascots built in**, all replaceable with your own art.
- **Calm and light.** A short animation every 8–20 seconds, then it sleeps.
  Static mode turns animation off.
- **Private.** No network access, no telemetry, and no keyboard or mouse
  monitoring. macOS never asks for special permissions.

Runs on **Windows 10/11**, **macOS 13+**, and **Linux**: X11, KDE Plasma 6
on Wayland, and GNOME on Wayland (Ubuntu 24.04 LTS and newer).

> **Status: pre-release.** PAWPRINT is being built now. This README describes
> how 1.0 will work. Today the shared core, the art compiler, and the
> backend-free commands (`pawprint explain`, `list`, `render`, `paths`,
> `--version`) work; the on-screen badge arrives with the platform backends.
> See [`ROADMAP.md`](ROADMAP.md) for progress and [`SPEC.md`](SPEC.md) for the
> design.

---

## Install

Install PAWPRINT **on each machine you want to identify**, the remote
machines, not the one you connect from. Installation is per user and needs
no admin rights.

**Linux and macOS**

```sh
curl -fsSL https://github.com/nealhardesty/pawprint/releases/latest/download/install.sh | sh
```

**Windows (PowerShell)**

```powershell
irm https://github.com/nealhardesty/pawprint/releases/latest/download/install.ps1 | iex
```

The installer downloads the right build for your system, verifies its
checksum, installs it, sets it to start when you log in, and starts it. The
badge should appear in the top-right corner of your primary display.

<details>
<summary>Installer options</summary>

| `install.sh` | `install.ps1` | Effect |
|---|---|---|
| `--version 1.2.0` | `-Version 1.2.0` | Install a specific version (default: latest) |
| `--archive ./pawprint-….tar.gz` | `-Archive .\pawprint-….zip` | Install from a downloaded archive, with no network access |
| `--no-autostart` | `-NoAutostart` | Don't start at login |
| `--no-start` | `-NoStart` | Don't start it now |
| `--prefix DIR` | — | Install somewhere other than `~/.local/bin` |
| `--uninstall [--purge]` | `-Uninstall [-Purge]` | Remove PAWPRINT (`purge` also deletes your config) |

Passing options through a pipe:

```sh
curl -fsSL https://github.com/nealhardesty/pawprint/releases/latest/download/install.sh | sh -s -- --no-autostart
```

```powershell
& ([scriptblock]::Create((irm https://github.com/nealhardesty/pawprint/releases/latest/download/install.ps1))) -NoAutostart
```
</details>

<details>
<summary>Manual install</summary>

Download the archive for your platform and its `.sha256` file from the
[releases page](https://github.com/nealhardesty/pawprint/releases), check the
hash, put `pawprint` (plus `pawprintw.exe` on Windows) somewhere on your
`PATH`, then run:

```sh
pawprint autostart on
pawprint
```

On macOS, a binary downloaded **with a browser** is quarantined by
Gatekeeper. Clear that with `xattr -d com.apple.quarantine ./pawprint`. The
curl installer doesn't have this problem.
</details>

Install locations: `~/.local/bin/pawprint` on Linux and macOS, and
`%LOCALAPPDATA%\Programs\pawprint\` on Windows (added to your user `PATH`).

---

## Everyday use

| You want to... | Do this |
|---|---|
| Start it | `pawprint` (it starts automatically at login after install) |
| Move it | Tray/menu-bar icon → **Move**, or `pawprint move`. Drag the badge, then let go. **Esc** cancels. |
| Put it back in the corner | `pawprint reset-position` |
| See why it chose that animal | `pawprint explain` |
| Pick a different animal | `pawprint list`, then `pawprint set mascot otter` |
| Change the label | `pawprint set label "BUILD-7"` (or `none` to hide it) |
| Make it bigger or smaller | `pawprint set size 128` (presets: 32, 64, 128) |
| Stop the animation | `pawprint set animation static` |
| Change the crate color | `pawprint set accent teal` (`pawprint list --accents` shows them all) |
| Preview a mascot | `pawprint preview walrus` |
| Stop it | `pawprint quit`, or tray → **Quit** |
| Stop it starting at login | `pawprint autostart off` |
| Find out what's wrong | `pawprint doctor` |

**Why "move mode"?** The badge lets every click pass through to whatever is
underneath, so it never blocks a button you need. To drag it, you first ask
it to hold still. Move mode ends when you let go, when you press Esc, or
after 30 seconds. Its position is anchored to the nearest corner or edge, so
it stays put when a remote session changes resolution.

### How it picks a mascot

1. **Your setting**, if you've chosen one.
2. **The animal in the name.** `owl`, `owl-02`, `prod-owl.example.com`, and
   `polar-bear-3` all match. Whole words only: `snowless` doesn't get an owl.
3. **Otherwise, a consistent pick.** The hostname is hashed to pick one of
   the 100 mascots and a crate color. The same name gives the same result on
   every install, and once chosen it is remembered, so adding new mascots
   later won't change your machines.

`pawprint explain` shows every step:

```
$ pawprint explain
hostname    prod-owl-02.lab.example
short name  prod-owl-02
label       PROD-OWL-02            (auto: short hostname, upper case)
mascot      owl                    (rule 4: token "owl")
accent      violet                 (from mascot)
size        64                     (default)
position    top-right +16+16       (dragged; saved in state file)
backend     wayland (layer-shell, KDE Plasma)
```

---

## Configuration

Everything works without a config file. When you want one,
`pawprint config init` writes a commented starter file, and
`pawprint config path` tells you where it lives:

| OS | Config file |
|---|---|
| Linux | `~/.config/pawprint/config.toml` |
| macOS | `~/Library/Application Support/pawprint/config.toml` |
| Windows | `%APPDATA%\pawprint\config.toml` |

The running badge picks up changes on its own. `pawprint reload` forces it.

```toml
mascot = "auto"          # "auto" or a mascot id from `pawprint list`
label = "auto"           # "auto" (short hostname) | "full" | "none" | any text
label_case = "upper"     # "upper" | "lower" | "as-is"
label_size = "auto"      # "auto" | "small" | "medium" | "large"
size = 64                # mascot size in pixels: 32, 64, 128 (any multiple of 32)
position = "top-right"   # top-left, top, top-right, left, center, right, bottom-left, bottom, bottom-right
offset = [16, 16]        # pixels in from that corner or edge
display = "primary"      # or an output name from `pawprint doctor`
animation = "idle"       # "idle" | "static"
interval = [8, 20]       # seconds between idle animations
accent = "auto"          # crate color: "auto", a name, or "#rrggbb"
tray = true              # show the tray / menu-bar icon
backend = "auto"         # leave this alone unless `pawprint doctor` says otherwise
```

### One config for every machine

Settings can target specific hosts, so you can copy **the same file to every
machine**. Each machine applies the sections that match its short hostname.
Patterns can use `*` and `?`.

```toml
size = 64

[hosts.buildbox]
mascot = "capybara"
label = "BUILD"

[hosts.lab-mini]
mascot = "cat-maine-coon-black"

[hosts."prod-*"]
accent = "cranberry"     # every production box gets a red crate
```

`pawprint set ...` writes into the current host's section for you and keeps
your comments and formatting.

A dragged position is saved separately (in PAWPRINT's state file) so your
config file is never rewritten behind your back. `pawprint reset-position`
clears it.

---

## Custom mascots

Every mascot can be replaced, and you can add your own. The easiest route is
a PNG sprite strip:

```sh
pawprint mascot export owl ./my-owl          # start from a built-in (PNG + mascot.toml)
# ...edit ./my-owl/sprite.png in Aseprite, LibreSprite, Piskel, etc...
pawprint mascot check ./my-owl               # validate it
```

Then copy the folder into the `mascots` folder next to your config file
(`pawprint paths` shows exactly where). **A folder with the same id as a
built-in replaces it. A new id adds a new mascot**, which you can select with
`pawprint set mascot <id>` or match by hostname through its `aliases`.

The rules, briefly:

- `sprite.png` is a horizontal strip of **32×32 frames**.
- Pixels are either fully opaque or fully transparent (no soft edges).
- Keep the face in the top 20 rows. The bottom 12 rows are behind the crate.
- `mascot.toml` names a rest frame and one or more short actions:

```toml
id = "my-owl"
name = "My Owl"
category = "birds"
aliases = ["hooter"]
accent = "violet"
sprite = "sprite.png"
rest = 0

[[actions]]
name = "blink"
frames = [1, 0]
duration_ms = 200
```

The built-in mascots are written in a **text pixel format** instead: pixel
grids in TOML, with shared body bases, reusable motions like `blink` and
`duck`, and props like `zzz`. Your own mascots can use it too, and it's the
way to contribute new built-ins. Both formats are documented in
[SPEC.md §12](SPEC.md#12-mascot-assets).

---

## Remote desktop notes

- **Install on the remote machine**, the one whose name you want to see.
- **Windows RDP:** each RDP session gets its own badge, started when you log
  in. A console session and an RDP session on the same machine each have
  their own.
- **Linux RDP and VNC (xrdp, Xvnc):** these sessions are usually X11 even if
  your desktop normally runs Wayland. PAWPRINT handles that automatically.
  If your session's window manager doesn't run autostart entries, start it
  with `pawprint`.
- **Parsec, Apollo/Moonlight, and Jump Desktop** stream the real screen, so
  the badge is in the stream. If a virtual display becomes primary while you
  are connected, the badge follows it.
- **Ubuntu / GNOME on Wayland:** works out of the box, including GNOME Remote
  Desktop's RDP and VNC (both "desktop sharing" and "remote login"). If you
  use **fractional scaling** (125%, 150%, ...), the badge can look blurry;
  `pawprint doctor` prints the one-line `gsettings` fix. The tray icon
  appears on Ubuntu. On GNOME without the AppIndicator extension, use the
  `pawprint` commands instead.

---

## Troubleshooting

Run **`pawprint doctor`** first. It reports the session type, which backend
was chosen and why, your displays and which one is primary, transparency
support, autostart status, and any config problems.

| Problem | Try |
|---|---|
| Nothing appears | `pawprint status`. If it isn't running, `pawprint run --verbose` shows errors in the terminal. |
| Black or gray box instead of transparency (Linux X11) | No compositor is running. PAWPRINT should switch to shaped mode automatically; `doctor` shows which mode is in use. |
| It's on the wrong monitor | `pawprint doctor` lists output names; set `display = "<name>"`. |
| Blurry on Ubuntu/GNOME with fractional scaling | Run `pawprint doctor`; it shows the `gsettings` command that makes it crisp. |
| It's behind a fullscreen game | Exclusive-fullscreen apps draw over everything. Use borderless windowed mode. |
| Wrong animal | `pawprint explain` shows the rule that fired; `pawprint set mascot <id>` overrides it. |
| Want a fresh fallback pick | `pawprint forget` clears this host's remembered choice. |
| "already running" but nothing visible | It's running in a different session (for example the console vs. RDP). `pawprint quit` from that session, or log out of it. |

Logs: `pawprint paths` shows the log file location.

---

## Uninstall

```sh
curl -fsSL https://github.com/nealhardesty/pawprint/releases/latest/download/install.sh | sh -s -- --uninstall
```

```powershell
& ([scriptblock]::Create((irm https://github.com/nealhardesty/pawprint/releases/latest/download/install.ps1))) -Uninstall
```

Your config and custom mascots are kept. Add `--purge` / `-Purge` to delete
them too.

---

## Development

PAWPRINT is Rust, with a small native window backend for each platform and
everything else written once in a shared, platform-independent core. Art is
text too: every mascot is a pixel grid in TOML, compiled at build time. Read [`SPEC.md`](SPEC.md)
before changing anything and [`ROADMAP.md`](ROADMAP.md) before proposing new
work. Contributor rules for humans and coding agents are in
[`AGENTS.md`](AGENTS.md).

### Prerequisites

| OS | Install |
|---|---|
| All | [rustup](https://rustup.rs). The pinned toolchain and targets install on first build. |
| Linux | `make`. Static musl release builds for x86_64 and aarch64 link with the `rust-lld` that rustup ships, so there is no cross toolchain to install. |
| macOS | Xcode Command Line Tools: `xcode-select --install` (provides `make`, `lipo`, `codesign`) |
| Windows | Visual Studio Build Tools (C++ workload) and GNU Make: `winget install ezwinports.make`. Run targets from PowerShell. |
| Publishing | [GitHub CLI](https://cli.github.com), authenticated with `gh auth login` |

### Make targets

**Everything goes through `make`.** Run `make` on its own to list targets.
The targets are the same on Linux, macOS, and Windows, and CI runs the same
ones.

| Target | What it does |
|---|---|
| `make help` | List targets (what `make` on its own runs) |
| `make build` | Debug build |
| `make run` | Build and run the badge in the foreground (`ARGS="--verbose"` passes flags) |
| `make preview MASCOT=owl` | Show one mascot cycling through its animations |
| `make test` | Unit tests, golden image tests, and mascot catalog validation |
| `make check-assets` | Validate the mascot catalog only |
| `make lint` | Format check and clippy (warnings are errors) |
| `make fmt` | Format the code |
| `make golden` | Regenerate golden images after an intentional rendering change |
| `make contact-sheet` | Render every mascot (labels hidden, grouped by body base) into `docs/gallery.png`; `ANIMATED=1` adds an animated GIF |
| `make art-export MASCOT=owl` | Export a mascot's layers and frames as PNGs for editing in a pixel-art tool |
| `make art-import MASCOT=owl PNG=eyes.png` | Convert an edited PNG layer back into the text pixel format |
| `make ci` | Everything CI checks (`lint` + `test`) |
| `make release` | Optimized build (universal binary on macOS, static musl on Linux) |
| `make package` | Release build, archive, and checksum into `dist/`. Fails if the catalog has fewer than 100 valid mascots or is missing a required one. |
| `make publish` | Upload `dist/` and the installers to the GitHub release for the current version |
| `make install` | Install your local release build for this user and turn on autostart |
| `make uninstall` | Remove the local install and autostart |
| `make doctor` | Run `pawprint doctor` from the local build |
| `make clean` | Remove build outputs |
| `make check-all-targets` | Type-check the workspace for every release target (Windows, macOS, Linux musl) from any OS; needs no linkers |
| `make spike-f22` | Rebuild the F22 spike: proves the Linux window, Wayland, D-Bus, and tray crates link into fully static musl binaries |

Targets for features that don't exist yet (see [`ROADMAP.md`](ROADMAP.md))
stop with `not implemented yet (Mx)`, naming the milestone that adds them.
`make check-assets` and `make test` report the catalog gates (at least 100
mascots, every showcase mascot present) without failing; only `make package`
enforces them.

### Releasing

Releases are built locally on each OS and uploaded to one GitHub release:

1. Bump `version` in the root `Cargo.toml`, commit, and tag `v<version>`.
2. On Linux, macOS, and Windows, run `make package && make publish`.
3. Test the install one-liners on a clean machine of each OS.

Details are in [SPEC.md §15](SPEC.md#15-packaging-release-and-install).

---

## License

[MIT](LICENSE), code and bundled art.
