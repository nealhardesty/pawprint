# PAWPRINT — Roadmap

This is the source of truth for **what does not exist yet**. When something
ships, move its facts into [`SPEC.md`](SPEC.md) and its user-facing docs into
[`README.md`](README.md), then check it off or delete it here. Read this
before proposing new work. If an idea isn't here, add it (with a reason)
before building it.

**Current state:** documentation only. No code, no art, no builds. The
repository is public.

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

- [ ] Cargo workspace per [SPEC §13](SPEC.md#13-repository-layout): `crates/pawprint-core`, `crates/pawprint-sprite`, `crates/pawprint`, `tools/pawdev`
- [ ] `rust-toolchain.toml` pinning stable and listing all release targets
- [ ] Root `Makefile`: `help` default, every target in [SPEC §14.2](SPEC.md#142-targets) present (stubs that fail clearly are fine for targets whose features don't exist yet), GNU Make 3.81 compatible
- [ ] CI workflow running `make ci` on `ubuntu-latest`, `macos-latest`, `windows-latest`
- [ ] `pawprint-core`:
  - [ ] hostname → short name → label pipeline ([SPEC §5.2](SPEC.md#52-labels))
  - [ ] alias table, `squash`, tokenizer, n-gram matcher ([SPEC §5.3](SPEC.md#53-choosing-a-mascot))
  - [ ] `fnv1a64` + `splitmix64` + HRW, with the frozen test vectors
  - [ ] accent palette and fallback accent
  - [ ] config parsing, `[hosts]` exact and glob precedence, warnings for unknown keys
  - [ ] state file read/atomic write
  - [ ] layout: anchor/offset ↔ rect, clamping, 3×3 snap, label/nameplate placement
  - [ ] move-mode controller (drag, constrain, snap, nudge, Esc, timeout)
  - [ ] animation scheduler with `next_deadline()`, weights, `when` (day/night)
  - [ ] 1× compositor, integer nearest-neighbor scaler, shared BGRA-premultiplied output + 1-bit mask
  - [ ] tray menu model
  - [ ] `Surface` trait + `FakeSurface`; event/effect loop tested end to end without a window
- [ ] `pawprint-sprite`: grid parser, palette mapping, layers and variants, layer ops, `extends`/recolor, templates, props, custom frames; PNG-mode mascots
- [ ] `build.rs` asset embedding + catalog validation
- [ ] Placeholder art (text format): crate, 5×7 font, one owl with rest + `blink`
- [ ] CLI skeleton (`clap`): `explain`, `list`, `render`, `paths`, `--version` working without any backend
- [ ] `make test` golden render of the placeholder owl at 32/64/128
- [ ] **Spike F22:** confirm a static musl Linux binary links with `x11rb` + pure-Rust `wayland-client` + `zbus`; choose the aarch64 cross approach (cross linker vs. `cargo-zigbuild`)

**Exit:** `make ci` is green on all three OSes. `pawprint explain --host prod-owl-02`
prints the right answer. `pawprint render owl --out owl.png` produces a crisp
badge.

---

## M1 — One-owl overlay prototype

Goal: the same badge on **Windows, macOS, Linux X11, KDE Plasma Wayland, and
Ubuntu GNOME Wayland**, meeting the backend contract
([SPEC §11](SPEC.md#11-platform-backends)). This is the milestone that can
change the design.

- [ ] Windows backend: layered topmost tool window, `UpdateLayeredWindow`,
      per-monitor DPI v2, work area, move mode via `HTCAPTION`;
      `pawprint.exe` + `pawprintw.exe`
- [ ] macOS backend: accessory app, non-activating `NSPanel` at status level,
      all Spaces, nearest-neighbor layer, `visibleFrame`, move mode
- [ ] Shared Linux module: `calloop` loop, scale helpers, backend selection
      ([SPEC §3.2](SPEC.md#32-linux-backend-selection-backend--auto))
- [ ] X11 backend: DOCK + ABOVE/STICKY/SKIP_*, empty input shape, ARGB visual
      with a compositor, bounding-shape fallback without one, override-redirect
      when no WM, RandR monitors, `_NET_WORKAREA`, move mode with pointer grab
- [ ] **GNOME profile** of the X11 backend, via XWayland ([SPEC §11.5](SPEC.md#115-gnome-wayland-ubuntu)),
      tested on Ubuntu 24.04 LTS and 26.04 LTS
- [ ] Wayland backend: layer-shell overlay, empty input region, margins from
      anchor/offset, `kde_output_order_v1` primary, fractional scale,
      move mode via implicit grab (fallback: full-output drag surface)
- [ ] Single instance per graphical session + IPC (`status`, `move`,
      `reset-position`, `reload`, `quit`), shared Unix transport for Linux and macOS
- [ ] `pawprint`, `run`, `move`, `reset-position`, `quit`, `status`, `doctor`
      (including GNOME fractional-scaling detection and fix)
- [ ] Display-change handling on every backend
- [ ] Facts to settle (update [SPEC §18](SPEC.md#18-facts-ledger)):
  - [ ] F3 `exclusive_zone = 0` vs. Plasma panels
  - [ ] F6 primary output on Plasma
  - [ ] F7 Wayland drag with implicit grab
  - [ ] F15 ad-hoc re-sign after `lipo`
  - [ ] F23 Windows virtual desktops
  - [ ] F24 layer-shell in KRdp/krfb streams
  - [ ] F25–F28, F31 GNOME stacking, input shape, primary, scaling, Xwayland on demand
- [ ] **Conditional:** if any GNOME check fails, build the Plan B presenter
      extension ([SPEC §11.5.3](SPEC.md#1153-plan-b-the-presenter-extension))
      and `pawprint gnome-extension install|remove`
- [ ] DRY review: no backend makes a layout, timing, or identity decision;
      each backend is within its size budget

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

- [ ] Master palette `assets/palette.toml` ([SPEC §12.9](SPEC.md#129-master-palette)), frozen at the end of M2; `pawdev` emits `palette.gpl` for pixel tools
- [ ] Final crate, label strip, and nameplate, tinted by accent ramps
- [ ] 15 props: `zzz`, `heart`, `bang`, `question`, `note`, `steam`, `sparkle`, `seed`, `leaf`, `bubble`, `star`, `sweat`, `puff`, `pebble`, `bamboo`
- [ ] 16 motion templates ([SPEC §12.4](SPEC.md#124-motion-templates))
- [ ] Bases needed by the showcase: `cat-sit`, `ungulate-head`, `bear`, `rodent-upright`, `bird-perched`, `pinniped`
- [ ] `make preview`, `make contact-sheet` (+ `ANIMATED=1`), `make art-export`, `make art-import`
- [ ] Art review checklist as a PR template ([SPEC §12.6](SPEC.md#126-production-process))

**Showcase 12**: each with ≥ 2 template actions, a signature action, and a
rare action:

- [ ] `owl`: blink / look-left / signature head turn / rare full head spin (night)
- [ ] `moose`: blink / ear-flick / signature heavy paddle-antler nod / rare: a bird lands on an antler
- [ ] `elk`: blink / ear-flick / signature head raise / rare: silent bugle with a `note` prop
- [ ] `marmot`: blink / duck / signature pop-up and double sniff / rare: whistle with `bang`
- [ ] `armadillo`: blink / peek / signature curl and half roll / rare: rolls off the crate and back
- [ ] `bear`: blink / doze / signature tiny yawn with paw on rim / rare: finds a `heart`-shaped honey drop
- [ ] `walrus`: blink / bob / signature whisker twitch and tusk bob / rare: balances a `pebble` on its nose
- [ ] `chipmunk`: blink / sniff / signature cheek-puff seed stash / rare: cheeks over-full, seed pops out
- [ ] `cat-maine-coon-black`: blink / ear-flick / signature amber side-eye and tail flick / rare: ruff fluffs up
- [ ] `cat-buff-tabby`: blink / content / signature melt into the crate until ears and cheeks remain / rare: loaf overflows the crate edge
- [ ] `cat-cow`: blink / look-right / signature paws the loose crate flap / rare: flap swings, `startle`
- [ ] `cat-tuxedo`: blink / tail-flick / signature tucks a white paw / rare: formal bow

- [ ] Readability review at 1× with labels covered: moose vs. elk, cow cat vs. tuxedo
- [ ] **Measure the 20-character label limit** against real hostnames and
      remote-client scaling at each size. Record the result in SPEC §4.4.

**Exit:** all 12 pass validation and two review rounds. The contact sheet
reads as one coherent set.

---

## M3 — Catalog of 100

Goal: meet the launch requirement of **≥ 100 complete, animated, selectable,
overridable mascots** ([SPEC Appendix A](SPEC.md#appendix-a-launch-catalog)),
each with a signature action.

Waves are grouped by base so siblings are reviewed side by side
([SPEC §12.8](SPEC.md#128-bases)). Each wave ends with a contact-sheet review.

- [ ] Wave 1: remaining cats (8) on `cat-sit`
- [ ] Wave 2: dogs (12) on `dog-sit`
- [ ] Wave 3: ungulates and bears (deer, bison, goat, llama, panda, polar-bear, koala)
- [ ] Wave 4: rodents and small quadrupeds (squirrel, mouse, hamster, beaver, fox, raccoon, red-panda, badger, skunk, opossum, hedgehog, otter)
- [ ] Wave 5: big cats, primates, and bespoke mammals (lion, tiger, monkey, sloth, rabbit, capybara, elephant, wolf)
- [ ] Wave 6: birds (15) on `bird-perched` and `bird-water`
- [ ] Wave 7: remaining aquatic (10; polar-bear is in wave 3) on `pinniped`, `sea-creature`, and bespoke
- [ ] Wave 8: critters and bugs (8) on `critter` and `bug`
- [ ] Wave 9: oddballs (8), all bespoke, each with a rare action (the CRT's `> hello_`, the ghost drifting through the crate, ...)
- [ ] Nocturnal set has night actions: owl, raccoon, moth, opossum, cat-black
- [ ] Alias audit: no collisions, no surprising whole-token matches
      against common hostname words (`db`, `web`, `prod`, `dev`, `test`,
      `build`, `mini`, `pro`, `lab`, `srv`, `node`, ...)
- [ ] Catalog stability golden test (200 synthetic hostnames) committed
- [ ] User mascot overrides and additions (text and PNG forms) from the config `mascots/` folder
- [ ] `pawprint mascot export` / `pawprint mascot check`
- [ ] `pawprint list` with source column
- [ ] `docs/gallery.png` and `docs/gallery.gif` embedded in the README

**Exit:** `make package` passes the catalog gates on a clean checkout, and
the full contact sheet passes review.

---

## M4 — Daily-driver features

Can run in parallel with M2/M3 once M1 passes.

- [ ] Tray / menu bar / StatusNotifierItem from the shared menu model ([SPEC §11.1.5](SPEC.md#1115-tray-and-autostart))
- [ ] `pawprint autostart on|off|status` (HKCU Run, LaunchAgent, XDG autostart)
- [ ] `pawprint set` / `unset` via `toml_edit`, `config init`, `config path`
- [ ] Config reload on `reload`, tray, and mtime change
- [ ] `explain --recompute`, `forget`
- [ ] Move mode polish: arrow-key nudging, Esc cancel, 30 s timeout, dashed outline
- [ ] Logging with size cap and rotation
- [ ] Power check against [SPEC §7.2](SPEC.md#72-power-budget) on each OS; record numbers in SPEC

**Exit:** a week of daily use across the owner's machines with no annoyances
worth filing.

---

## M5 — Packaging and 1.0

- [ ] `make release`: universal macOS binary, static musl Linux (x86_64 + aarch64), Windows MSVC
- [ ] `make package`: archives + `.sha256` sidecars into `dist/`
- [ ] `make publish`: create/update GitHub release via `gh`, upload `dist/` + installers
- [ ] `install.sh` (POSIX sh) and `install.ps1` (PS 5.1 + 7) per [SPEC §15](SPEC.md#15-packaging-release-and-install), including `--archive`, `--uninstall`, `--purge`
- [ ] `make install` / `make uninstall` for local builds
- [ ] Clean-machine install test on each OS, including Ubuntu 24.04 and 26.04
- [ ] Full [remote compatibility matrix](SPEC.md#164-remote-compatibility-matrix), results in release notes
- [ ] README reviewed against reality: every command, path, and screenshot

**Exit:** `v1.0.0` published. The install one-liners work on clean
Windows, macOS, and Linux machines.

---

## After 1.0

Ideas worth doing, roughly by value. None are commitments.

| Idea | Why | Notes |
|---|---|---|
| **All-displays mode** (`display = "all"`) | Remote clients that show one secondary monitor alone | One surface per output; same position model per display |
| **Respect OS "reduce motion"** | Accessibility | Windows `SPI_GETCLIENTAREAANIMATION`, macOS `accessibilityDisplayShouldReduceMotion`, KDE/GNOME animation settings → static |
| **Shy mode** | See what's under the badge without moving it | Fade when the pointer hovers. Needs global pointer position, which Wayland doesn't give; maybe Windows/macOS/X11 only. |
| **Modifier-drag** | Move without opening move mode | Hold a configurable modifier over the badge. Same Wayland caveat. |
| **Release builds in CI** | Fewer manual steps | Public repo means free runners; keep local `make publish` as the fallback |
| **Code signing and notarization** | No Gatekeeper/SmartScreen friction for browser downloads | Needs an Apple Developer ID and a Windows signing certificate |
| **Windows ARM64 build** | Snapdragon laptops and ARM VMs | `aarch64-pc-windows-msvc` |
| **Package managers** | Discoverability | Homebrew tap, Scoop/winget manifest, AUR, a `.deb`/PPA for Ubuntu. Flatpak's sandbox doesn't fit a desktop overlay. |
| **Community mascot packs** | More animals without bloating the binary | Directory-of-mascots zip installed into the config `mascots/` folder; `pawprint mascot install <zip>` |
| **Seasonal crate skins** | Delight | Opt-in only; must never change identity cues (mascot, accent) |
| **Environment cue** | Know prod from dev at a glance | E.g. a striped "hazard" crate for hosts matching a pattern; already partly possible with `accent` per glob |
| **Session-type hint** | Know you're in RDP vs. console | A tiny glyph on the crate (`R`, `V`, `C`); detection is platform-specific |
| **Other Wayland compositors in the matrix** | Sway, Hyprland, COSMIC users | Layer-shell path should already work; needs testers |

### Explicitly not planned

- Reacting to keystrokes or mouse clicks: it would need input-monitoring
  permissions, and PAWPRINT's privacy promise rules it out.
- Network features (fleet sync, phoning home). Copy the config file
  instead.
- A settings GUI. The CLI, the tray, and one TOML file cover it.

---

## Open questions

Decisions still needed. Each has a recommendation.

1. **Linux aarch64 at 1.0, or later?** *Recommendation:* include it if the
   M0 cross-build spike is easy (it's the same code); otherwise move it after
   1.0.
2. **Wayland drag fallback.** If F7 fails on Plasma, is a full-output
   transparent drag surface acceptable, or should Wayland move mode be
   keyboard-only? *Recommendation:* full-output surface; decide after the
   M1 spike.

Settled since the first draft: the repository is public; GNOME on Wayland
(Ubuntu) is a launch target; backends share everything through the core;
art is produced through the text pipeline in SPEC §12; the label limit is 20,
to be measured in M2.
