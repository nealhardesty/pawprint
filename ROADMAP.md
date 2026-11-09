# PAWPRINT — Roadmap

This is the source of truth for **what does not exist yet**. When something
ships, move its facts into [`SPEC.md`](SPEC.md) and its user-facing docs into
[`README.md`](README.md), then check it off or delete it here. Read this
before proposing new work. If an idea isn't here, add it (with a reason)
before building it.

**Current state:** everything in [`SPEC.md`](SPEC.md) is implemented
and tested in CI on Linux, macOS, and Windows: all five backends (Windows,
macOS, X11 with its GNOME profile, Wayland), the tray and autostart on every
OS, the full catalog of 100 mascots, the power checks, release builds,
packaging, and the installers. What remains needs the owner: a first
published release, the clean-machine install one-liners, checks on real
KDE Plasma and Ubuntu GNOME desktops, the remote compatibility matrix, and
a week of daily use. Those items are unchecked below and marked
**(owner)**. The repository is public.

---

## Strategy

The expensive, uncertain part of PAWPRINT is not the art. It's whether a
transparent, click-through, always-on-top window behaves on **five
desktops** (Windows, macOS, Linux X11, KDE Plasma Wayland, Ubuntu GNOME
Wayland) **and** survives five remote-desktop clients. So the order is:

1. **Prove the overlay with one owl** on every desktop, through real remote
   sessions, before drawing more than a few sprites.
2. **Build the art pipeline, then the showcase 12** on top of it.
3. **Fill the catalog to 100** in waves, grouped by body base.
4. **Package it** so install is a one-liner.

Throughout: **write everything once.** Backends translate native events and
nothing else; logic lives in `pawprint-core` ([SPEC §10.6](SPEC.md#106-code-sharing-dry)).
Reviews reject a second copy of any logic.

Each milestone has exit criteria. Don't start the next milestone's risky
work until the current one's criteria pass. Small independent tasks can run
ahead.

```
M0 Foundations ──► M1 One-owl overlay ──► M2 Art pipeline + showcase 12 ──► M3 Catalog 100 ──► M5 1.0
        │                  │                                                       ▲
        └──────────────────┴──────────────► M4 Daily-driver features ──────────────┘
```

---

## M0 — Foundations

Goal: a workspace where the pure logic is done and tested, and every
automation path exists, before any window opens.

- [x] Cargo workspace per [SPEC §13](SPEC.md#13-repository-layout): `crates/pawprint-core`, `crates/pawprint-sprite`, `crates/pawprint`, `tools/pawdev`
- [x] `rust-toolchain.toml` pinning stable and listing all release targets
- [x] Root `Makefile`: `help` default, every target in [SPEC §14.2](SPEC.md#142-targets) present, GNU Make 3.81 compatible
- [x] CI workflow running `make ci` on `ubuntu-latest`, `macos-latest`, `windows-latest`
- [x] `pawprint-core`:
  - [x] hostname → short name → label pipeline ([SPEC §5.2](SPEC.md#52-labels))
  - [x] alias table, `squash`, tokenizer, n-gram matcher ([SPEC §5.3](SPEC.md#53-choosing-a-mascot))
  - [x] `fnv1a64` + `splitmix64` + HRW, with the frozen test vectors
  - [x] accent palette and fallback accent
  - [x] config parsing, `[hosts]` exact and glob precedence, warnings for unknown keys
  - [x] state file read/atomic write
  - [x] layout: anchor/offset ↔ rect, clamping, 3×3 snap, label/nameplate placement
  - [x] move-mode controller (drag, constrain, snap, nudge, Esc, timeout)
  - [x] animation scheduler with `next_deadline()`, weights, `when` (day/night)
  - [x] 1× compositor, integer nearest-neighbor scaler, shared BGRA-premultiplied output + 1-bit mask
  - [x] tray menu model
  - [x] `Surface` trait + `FakeSurface`; event/effect loop tested end to end without a window
- [x] `pawprint-sprite`: grid parser, palette mapping, layers and variants, layer ops, `extends`/recolor, templates, props, custom frames; PNG-mode mascots
- [x] `build.rs` asset embedding + catalog validation
- [x] Placeholder art (text format): crate, 5×7 font, one owl with rest + `blink`
- [x] CLI skeleton (`clap`): `explain`, `list`, `render`, `paths`, `--version` working without any backend
- [x] `make test` golden render of the placeholder owl at 32/64/128
- [x] **Spike F22:** confirm a static musl Linux binary links with `x11rb` + pure-Rust `wayland-client` + `zbus`; choose the aarch64 cross approach (cross linker vs. `cargo-zigbuild`)

**Exit:** `make ci` is green on all three OSes. `pawprint explain --host prod-owl-02`
prints the right answer. `pawprint render owl --out owl.png` produces a crisp
badge.

Spike F22 outcome: static musl binaries link for x86_64 and aarch64 with
rustup's `rust-lld`; no cross linker or `cargo-zigbuild` (SPEC §14.4, F22).

---

## M1 — One-owl overlay prototype

Goal: the same badge on **Windows, macOS, Linux X11, KDE Plasma Wayland, and
Ubuntu GNOME Wayland**, meeting the backend contract
([SPEC §11](SPEC.md#11-platform-backends)). This is the milestone that can
change the design.

- [x] Windows backend: layered topmost tool window, `UpdateLayeredWindow`,
      per-monitor DPI v2, work area, move mode via `HTCAPTION`;
      `pawprint.exe` + `pawprintw.exe`
- [x] macOS backend: accessory app, non-activating `NSPanel` at status level,
      all Spaces, nearest-neighbor layer, `visibleFrame`, move mode
- [x] Shared Linux module: `calloop` loop, scale helpers, backend selection
      ([SPEC §3.2](SPEC.md#32-linux-backend-selection-backend--auto))
- [x] X11 backend: DOCK + ABOVE/STICKY/SKIP_*, empty input shape, ARGB visual
      with a compositor, bounding-shape fallback without one, override-redirect
      when no WM, RandR monitors, `_NET_WORKAREA`, move mode with pointer grab
      (tested under Xvfb bare, with `xcompmgr`, and with `openbox`)
- [ ] **(owner)** **GNOME profile** of the X11 backend via XWayland
      ([SPEC §11.5](SPEC.md#115-gnome-wayland-ubuntu)), checked on real
      Ubuntu 24.04 LTS and 26.04 LTS. *Implemented, selected automatically,
      and covered by `make test-x11`; only the runs on real Ubuntu remain.*
- [x] Wayland backend: layer-shell overlay, empty input region, margins from
      anchor/offset, `kde_output_order_v1` primary, fractional scale,
      move mode (implicit-grab drag failed on sway, F7; the full-area drag
      surface is the design now). Tested under headless sway; the Plasma
      check is the owner's (facts F3, F6 below).
- [x] Single instance per graphical session + IPC (`status`, `move`,
      `reset-position`, `reload`, `quit`): a Unix socket on Linux and macOS,
      a named pipe on Windows, with a session key per OS
- [x] `pawprint`, `run`, `move`, `reset-position`, `quit`, `status`, `doctor`
      (including GNOME fractional-scaling detection and fix)
- [x] Display-change handling on every backend
- [x] Display-server tests: `make test-x11`, `make test-wayland`, and a Linux
      CI job that runs them
- [ ] **(owner)** Facts to settle (update [SPEC §18](SPEC.md#18-facts-ledger)):
  - [ ] **(owner)** F3 `exclusive_zone = 0` vs. Plasma panels (confirmed on sway)
  - [ ] **(owner)** F6 primary output on Plasma
  - [x] F7 Wayland drag with implicit grab (refuted on sway 1.9: press-relative coordinates)
  - [x] F15 ad-hoc re-sign after `lipo` (confirmed on `macos-latest` in M5)
  - [ ] **(owner)** F23 Windows virtual desktops
  - [ ] **(owner)** F24 layer-shell in KRdp/krfb streams
  - [ ] **(owner)** F25–F28, F31 GNOME stacking, input shape, primary, scaling, Xwayland on demand
- [ ] **Conditional, only if an owner GNOME check fails:** build the Plan B
      presenter extension ([SPEC §11.5.3](SPEC.md#1153-plan-b-the-presenter-extension))
      and `pawprint gnome-extension install|remove` (the command exists,
      hidden from `--help`, and says it is not implemented)
- [x] DRY review: no backend makes a layout, timing, or identity decision;
      each backend is within its size budget (*X11 and Wayland reviewed:
      about 760 and 800 code lines after rustfmt, no decisions; Wayland's
      anchor choice reuses `layout::snap`*)
- [x] macOS hostname: read `HostName`/`LocalHostName` through the
      SystemConfiguration API instead of `scutil --get` (M0 stopgap,
      SPEC §5.1)
- [x] Local clock hour for `Now::local_hour` on each platform (the core takes
      it as an input; SPEC §10.1): `localtime_r` on Unix, `GetLocalTime` on Windows
- [x] `pawprintw.exe` second `[[bin]]` on Windows (SPEC §11.1.6)
- [x] Move-mode drag events in physical desktop coordinates on every backend
      (the core's coordinate space, SPEC §6.1)

**Exit:** [SPEC §16.3](SPEC.md#163-spike-exit-criteria-m1-one-owl-prototype)
passes on all five desktops, **and** the owner has run a first pass of the
[remote matrix](SPEC.md#164-remote-compatibility-matrix) with at least one
client per host OS. Any backend that fails gets a written decision in SPEC
(workaround, documented limitation, or design change) before M2 starts.

---

## M2 — Art pipeline and the showcase 12

Goal: prove the text-art pipeline ([SPEC §12](SPEC.md#12-mascot-assets)) end
to end on the mascots that matter most.

**Pipeline**

- [x] Master palette `assets/palette.toml` ([SPEC §12.9](SPEC.md#129-master-palette)), frozen; `make palette` emits `palette.gpl` for pixel tools
- [x] Final crate, label strip, and nameplate, tinted by accent ramps
- [x] 15 props: `zzz`, `heart`, `bang`, `question`, `note`, `steam`, `sparkle`, `seed`, `leaf`, `bubble`, `star`, `sweat`, `puff`, `pebble`, `bamboo`
- [x] 16 motion templates ([SPEC §12.4](SPEC.md#124-motion-templates)), as 17 files (`look-left` and `look-right` are separate)
- [x] All 14 bases ([SPEC §12.8](SPEC.md#128-bases)), not only the showcase's, so the M3 waves can start in parallel
- [x] Bases, templates, and props embedded in the bundle for text-format user mascots
- [x] Layer `follows` so head motions carry the face
- [x] `make contact-sheet` (+ `ANIMATED=1`), `make building-blocks`, `make art-export`, `make art-import`
- [x] `make preview` / `pawprint preview` on Linux X11 and Wayland (M4; the Windows and macOS hooks are listed under M4)
- [x] Art review checklist as a PR template ([SPEC §12.6](SPEC.md#126-production-process))
- [x] Style guide with worked examples: [`docs/ART.md`](docs/ART.md)
- [x] Reference owl on `bird-perched` (replaces the M0 placeholder)

**Showcase 12**: each with ≥ 2 template actions, a signature action, and a
rare action:

- [x] `owl`: blink / look-left / signature head turn / rare full head spin (night)
- [x] `moose`: blink / ear-flick / signature heavy paddle-antler nod / rare: a bird lands on an antler
- [x] `elk`: blink / ear-flick / signature head raise / rare: silent bugle with a `note` prop
- [x] `marmot`: blink / duck / signature pop-up and double sniff / rare: whistle with `bang`
- [x] `armadillo`: blink / peek / signature curl and half roll / rare: rolls off the crate and back
- [x] `bear`: blink / doze / signature tiny yawn with paw on rim / rare: finds a `heart`-shaped honey drop
- [x] `walrus`: blink / bob / signature whisker twitch and tusk bob / rare: balances a `pebble` on its nose
- [x] `chipmunk`: blink / sniff / signature cheek-puff seed stash / rare: cheeks over-full, seed pops out
- [x] `cat-maine-coon-black`: blink / ear-flick / signature amber side-eye and tail flick / rare: ruff fluffs up
- [x] `cat-buff-tabby`: blink / content / signature melt into the crate until ears and cheeks remain / rare: loaf overflows the crate edge
- [x] `cat-cow`: blink / look-right / signature paws the loose crate flap / rare: flap swings, `startle`
- [x] `cat-tuxedo`: blink / tail-flick / signature tucks a white paw / rare: formal bow

- [x] Readability review at 1× with labels covered: moose vs. elk, cow cat vs. tuxedo (all showcase mascots also have `doze` for night, per SPEC §12.5)
- [x] **Measure the 20-character label limit** against real hostnames and
      remote-client scaling at each size. Record the result in SPEC §4.4.
      (*Done in M4: 20 stays; long names are now cut in the middle so their
      distinguishing end survives.*)

**Exit:** all 12 pass validation and two review rounds. The contact sheet
reads as one coherent set.

---

## M3 — Catalog of 100

Goal: meet the launch requirement of **≥ 100 complete, animated, selectable,
overridable mascots** ([SPEC Appendix A](SPEC.md#appendix-a-launch-catalog)),
each with a signature action.

Waves are grouped by base so siblings are reviewed side by side
([SPEC §12.8](SPEC.md#128-bases)). Each wave ends with a contact-sheet review.

- [x] Wave 1: remaining cats (8) on `cat-sit`
- [x] Wave 2: dogs (12) on `dog-sit`
- [x] Wave 3: ungulates and bears (deer, bison, goat, llama, panda, polar-bear, koala)
- [x] Wave 4: rodents and small quadrupeds (squirrel, mouse, hamster, beaver, fox, raccoon, red-panda, badger, skunk, opossum, hedgehog, otter)
- [x] Wave 5: big cats, primates, and bespoke mammals (lion, tiger, monkey, sloth, rabbit, capybara, elephant, wolf)
- [x] Wave 6: birds (15) on `bird-perched` and `bird-water`
- [x] Wave 7: remaining aquatic (10; polar-bear is in wave 3) on `pinniped`, `sea-creature`, and bespoke
- [x] Wave 8: critters and bugs (8) on `critter` and `bug`
- [x] Wave 9: oddballs (8), all bespoke, each with a rare action (the CRT's `> hello_`, the ghost drifting through the crate, ...)
- [x] Nocturnal set has night actions: owl, raccoon, moth, opossum, cat-black, and the ghost (added in wave 9)
- [x] Alias audit: no collisions, no surprising whole-token matches
      against common hostname words (`db`, `web`, `prod`, `dev`, `test`,
      `build`, `mini`, `pro`, `lab`, `srv`, `node`, ...). Removed `bug`,
      `lab`, `grey`, `monitor`, `tv`, `terminal`, `disk`, and `bot`
      ([SPEC Appendix A](SPEC.md#appendix-a-launch-catalog))
- [x] Catalog stability golden test (200 synthetic hostnames) committed
- [x] User mascot overrides and additions (text and PNG forms) from the config `mascots/` folder;
      embed the bases, motion templates, and props in the bundle so text-form
      user mascots can use them (SPEC §12.13)
- [x] `pawprint mascot export` / `pawprint mascot check`
- [x] `pawprint list` with source column
- [x] `docs/gallery.png` and `docs/gallery.gif` embedded in the README

**Exit:** `make package` passes the catalog gates on a clean checkout, and
the full contact sheet passes review.

---

## M4 — Daily-driver features

Can run in parallel with M2/M3 once M1 passes.

- [x] Tray plumbing from the shared menu model: `shell::Tray`, `Shell::set_tray`, `Shell::menu` (one command path with IPC; Size and Animation saved like `pawprint set`, SPEC §8.6)
- [x] Linux StatusNotifierItem via `ksni` for X11, Wayland, and GNOME/AppIndicator, tested over D-Bus in `make test-x11` ([SPEC §11.4](SPEC.md#114-linux-wayland-kde-plasma-6))
- [x] Windows tray (`Shell_NotifyIconW`) and macOS menu bar (`NSStatusItem`) on the same `shell::Tray` ([SPEC §11.1.5](SPEC.md#1115-tray-and-autostart), §11.2)
- [x] `pawprint autostart on|off|status` and the `autostart` interface, with XDG autostart on Linux
- [x] Autostart on Windows (HKCU Run → `pawprintw.exe run`) and macOS (LaunchAgent): fill the hooks in `pawprint::autostart`
- [x] `pawprint preview` on Windows and macOS: fill the `backend::{windows,macos}::preview` hooks (the shell's preview mode is shared)
- [x] `pawprint set` / `unset` via `toml_edit`, `config init`, `config path`
- [x] Config reload on `reload`, tray, and mtime change
- [x] `explain --recompute`, `forget`
- [x] Move mode polish: arrow-key nudging, Esc cancel, 30 s timeout, dashed outline (the outline is drawn by the core, so every backend shows it)
- [x] Logging with size cap and rotation (done in M1; dependencies now log from warn up)
- [x] Power check on Linux: `make power-check`, numbers in [SPEC §7.2](SPEC.md#72-power-budget) (it found and fixed a twice-a-second wakeup, F36)
- [x] Power check on Windows and macOS (on macOS, AppKit's own threads keep a small static-mode floor; the check budgets our share, SPEC §7.2)

- [ ] **(owner)** A week of daily use across the owner's machines with no
      annoyances worth filing

**Exit:** that week of daily use.

---

## M5 — Packaging and 1.0

- [x] `make release`: universal macOS binary (re-signed ad hoc, F15), static musl Linux (x86_64 + aarch64), Windows MSVC; every binary checked and run ([SPEC §14.5](SPEC.md#145-release-builds))
- [x] `make package`: archives + `.sha256` sidecars into `dist/`
- [x] `make publish`: create a draft GitHub release via `gh` and upload `dist/` + installers; refuses a dirty tree, a tag off `HEAD`, or a `dist/` packaged from another commit; `make release-finalize` publishes the draft once all ten assets are there; `DRY_RUN=1` of both tested in CI
- [x] Release workflow: a `v*` tag push runs `make package` + `make publish` on each OS and `make release-finalize` (`.github/workflows/release.yml`)
- [x] First real release: [`v1.0.0`](https://github.com/nealhardesty/pawprint/releases/tag/v1.0.0), built and published by the release workflow; the Linux one-liner verified against it
- [x] `install.sh` (POSIX sh) and `install.ps1` (PS 5.1 + 7) per [SPEC §15](SPEC.md#15-packaging-release-and-install), including `--archive`, `--uninstall`, `--purge`
- [x] `make install` / `make uninstall` for local builds
- [x] `make test-install`: the installers end to end under sh, dash, bash, PowerShell 5.1, and PowerShell 7, in CI on all three OSes ([SPEC §15.7](SPEC.md#157-installer-tests))
- [x] `make test-install` requires autostart on every OS
- [ ] **(owner)** Clean-machine install test on each OS, including Ubuntu 24.04 and 26.04 (needs a published release)
- [ ] **(owner)** Full [remote compatibility matrix](SPEC.md#164-remote-compatibility-matrix), results in release notes (real machines)
- [ ] **(owner)** README reviewed against reality: every command, path, and screenshot, at release (install and development sections done in M5)

**Exit:** `v1.0.0` published. The install one-liners work on clean
Windows, macOS, and Linux machines.

---

## Next: v1.1.1

- [ ] **Label: drop the side margins.** The owner chose this; it wasn't
  finished in time for v1.1.0. Keep the 1 art-px top and bottom margin,
  but let the label use the strip's full width (28P instead of 26P).
  - **Effect:** 7 micro glyphs fit at size 32 (`PRODOWL` instead of
    `PRODOW`), and 9-character names such as `ARMADILLO` at size 64 use
    the 5×7 font instead of the micro font.
  - **Code:** the fit rule in `crates/pawprint-core/src/layout.rs`, plus its
    tests and the expected-results table.
  - **Then:** update SPEC §4.4 and regenerate the affected goldens
    (`make golden`). Release as v1.1.1.

## Settings window and fullscreen hiding

Both are implemented and tested (SPEC §6.5 and §8.7): `fullscreen =
"hide" | "show"` on every backend, and `pawprint settings` (tray:
**Settings…**) with a control for every config key in any table. What
remains needs the owner's machines:

- [ ] **(owner)** F39: a fullscreen Space on a real Mac (Jump Desktop or
      Screens in full screen) hides the badge, and `fullscreen = "show"`
      keeps it
- [ ] **(owner)** F41: `mstsc` going full screen in place (Ctrl+Alt+Break)
      and back hides and shows the local badge; a multi-monitor RDP session
      hides it on every display
- [ ] **(owner)** F42: a fullscreen window on Plasma covers the badge
- [ ] **(owner)** F40: a fullscreen native Wayland app on Ubuntu GNOME
      lowers the badge
- [ ] **(owner)** The settings window on each desktop: opened from the
      tray, typing, scrolling the mascot grid, a 200% display, and a
      config edited by hand while it is open

## After 1.0

Ideas worth doing, roughly by value. None are commitments.

| Idea | Why | Notes |
|---|---|---|
| **All-displays mode** (`display = "all"`) | Remote clients that show one secondary monitor alone | One surface per output; same position model per display |
| **Respect OS "reduce motion"** | Accessibility | Windows `SPI_GETCLIENTAREAANIMATION`, macOS `accessibilityDisplayShouldReduceMotion`, KDE/GNOME animation settings → static |
| **Shy mode** | See what's under the badge without moving it | Fade when the pointer hovers. Needs global pointer position, which Wayland doesn't give; maybe Windows/macOS/X11 only. |
| **Modifier-drag** | Move without opening move mode | Hold a configurable modifier over the badge. Same Wayland caveat. |
| **Code signing and notarization** | No Gatekeeper/SmartScreen friction for browser downloads | Needs an Apple Developer ID and a Windows signing certificate |
| **Windows ARM64 build** | Snapdragon laptops and ARM VMs | `aarch64-pc-windows-msvc` |
| **Package managers** | Discoverability | Homebrew tap, Scoop/winget manifest, AUR, a `.deb`/PPA for Ubuntu. Flatpak's sandbox doesn't fit a desktop overlay. |
| **Community mascot packs** | More animals without bloating the binary | Directory-of-mascots zip installed into the config `mascots/` folder; `pawprint mascot install <zip>` |
| **Seasonal crate skins** | Delight | Opt-in only; must never change identity cues (mascot, accent) |
| **Environment cue** | Know prod from dev at a glance | E.g. a striped "hazard" crate for hosts matching a pattern; already partly possible with `accent` per glob |
| **Session-type hint** | Know you're in RDP vs. console | A tiny glyph on the crate (`R`, `V`, `C`); detection is platform-specific |
| **Key repeat in the settings window on Wayland** | Holding Backspace deletes one character | Needs a timer from the core (`repeat_info`), like the move-mode timeout |
| **Paste into the settings window's fields** | Long labels and colors are faster to paste | The clipboard is a different API on each OS; the fields are short |
| **Other Wayland compositors in the matrix** | Sway, Hyprland, COSMIC users | Layer-shell path should already work; needs testers |

### Explicitly not planned

- Reacting to keystrokes or mouse clicks: it would need input-monitoring
  permissions, and PAWPRINT's privacy promise rules it out.
- Network features (fleet sync, phoning home). Copy the config file
  instead.

---

## Open questions

Decisions still needed. Each has a recommendation.

None open right now.

Settled since the first draft: Wayland move mode drags on a full-area
transparent surface, because the implicit-grab drag failed on sway (F7,
SPEC §11.4); Linux aarch64 ships at 1.0 (the M0 spike
showed it cross-links with rustup's `rust-lld` alone); the repository is public; GNOME on Wayland
(Ubuntu) is a launch target; backends share everything through the core;
art is produced through the text pipeline in SPEC §12; the label limit is 20
(measured in M4; long names are cut in the middle, SPEC §4.4).
