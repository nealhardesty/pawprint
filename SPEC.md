# PAWPRINT — Specification

This is the single source of truth for **how PAWPRINT works**: product rules,
architecture, formats, platform behavior, build and release mechanics, and the
facts we learned the hard way. Read it before changing anything. If code and
this document disagree, one of them is a bug; fix whichever is wrong in the
same change.

- What does not exist yet is in [`ROADMAP.md`](ROADMAP.md).
- How to install and use it is in [`README.md`](README.md).

**Status of this document:** M0 (foundations) is implemented: the shared
core, the sprite compiler, embedded placeholder art, and the backend-free CLI
commands. M1 is in progress: the shared runtime (shell, IPC, single
instance, doctor) and the Linux backends (X11 with its GNOME profile, and
Wayland) are implemented and tested against real display servers
([16.1.1](#1611-display-server-tests-linux)); the Windows and macOS backends
are next. The repository is public. Facts marked
**(verify)** come from platform documentation or prior experience and have not
yet been confirmed by a PAWPRINT spike. When a spike confirms or refutes one,
update the [Facts ledger](#18-facts-ledger) and drop the marker.

---

## Contents

1. [Product](#1-product)
2. [Names and identifiers](#2-names-and-identifiers)
3. [Supported environments](#3-supported-environments)
4. [The badge](#4-the-badge)
5. [Host identity: hostname to label and mascot](#5-host-identity-hostname-to-label-and-mascot)
6. [Placement and move mode](#6-placement-and-move-mode)
7. [Animation and power](#7-animation-and-power)
8. [Configuration](#8-configuration)
9. [Command-line interface](#9-command-line-interface)
10. [Runtime architecture](#10-runtime-architecture)
11. [Platform backends](#11-platform-backends)
12. [Mascot assets](#12-mascot-assets)
13. [Repository layout](#13-repository-layout)
14. [Build system](#14-build-system)
15. [Packaging, release, and install](#15-packaging-release-and-install)
16. [Testing and acceptance](#16-testing-and-acceptance)
17. [Accounts and external services](#17-accounts-and-external-services)
18. [Facts ledger](#18-facts-ledger)
19. [Appendix A: launch catalog](#appendix-a-launch-catalog)
20. [Appendix B: accent palette](#appendix-b-accent-palette)

---

## 1. Product

### 1.1 Problem

People who remote into many machines (Windows, macOS, and Linux desktops,
through RDP, VNC, Jump Desktop, Parsec, Apollo/Moonlight, and others) lose
track of which machine they are looking at. Window titles and wallpapers are
easy to miss or look the same everywhere.

### 1.2 Solution

PAWPRINT is a small, always-on-top, click-through, pixel-art **badge** that
runs inside the graphical session of each machine. It shows a mascot animal
sitting in a shipping crate stamped with the machine's short hostname. A host
named `moose` shows a moose. A host named `owl` shows an owl that blinks and
turns its head now and then. Hosts with no obvious animal still get a stable,
deterministic mascot, so every machine looks different.

Because the badge runs *on the machine being identified* and is drawn into
its desktop, it shows up in every remote client with no client-side
integration.

### 1.3 Principles

1. **Zero-config correct.** A fresh install on a host named `owl` shows an owl
   labeled `OWL` in the top-right corner of the primary display. No settings
   are needed to get there.
2. **Never in the way.** The badge is click-through, never takes keyboard
   focus, never appears in the taskbar, Dock, or Alt-Tab, and is moved only
   when the user explicitly asks to move it.
3. **Calm and cheap.** One brief idle animation every 8–20 seconds at about
   5 FPS. Between animations the process sleeps with no timers running.
   Static mode turns animation off entirely.
4. **Everything overridable.** Every mascot, label, size, position, and color
   can be changed in one TOML file, and that file can be shared across a whole
   fleet.
5. **Private by construction.** No network access, no telemetry, no input
   monitoring. PAWPRINT never needs Accessibility, Input Monitoring, or Screen
   Recording permission on macOS. (Unlike keystroke-reactive desktop pets,
   PAWPRINT's animation is self-timed.)
6. **One binary, no runtime dependencies.** All bundled art is embedded in the
   executable. Install is a single file plus an autostart entry.

### 1.4 Launch requirements (1.0)

| Area | Requirement |
|---|---|
| Desktops | Windows 10/11, macOS 13+, Linux X11, Linux Wayland on KDE Plasma 6 and on GNOME (Ubuntu 24.04 LTS and newer) |
| Remote clients | RDP, Jump Desktop, VNC, Parsec, Apollo/Moonlight. The project owner runs end-to-end remote testing (see [16.4](#164-remote-compatibility-matrix)). |
| Mascots | **At least 100 animated, selectable, overridable mascots bundled**, including the 12-mascot showcase set ([Appendix A](#appendix-a-launch-catalog)), each with a signature action. Art is complete and original ([12](#12-mascot-assets)). `make package` refuses to build otherwise. |
| Placement | One badge on the primary display. Draggable through move mode. Position remembered across restarts, reconnects, and resolution changes. |
| Look | Transparent, always-on-top pixel art. Default mascot size 64 logical pixels; presets 32 and 128. Label sized separately. |
| Identity | Short hostname label by default. Custom label and mascot override. Deterministic fallback mascot for unmatched names. |
| Runtime | Rust. Small native backends over a shared core: everything that isn't a platform call is written once ([10.6](#106-code-sharing-dry)). Pregenerated sprites. Brief idle animations. Static mode. |
| Build | Root GNU Makefile with identical public targets on all three OSes. |
| Install | Per-user, no admin. `install.sh` (Linux, macOS) and `install.ps1` (Windows). Autostart at graphical login. |

### 1.5 Non-goals for 1.0

- A badge on every monitor. 1.0 shows one badge on the primary display.
- Reacting to keyboard or mouse activity.
- Seamless or RemoteApp-style remote windows. PAWPRINT needs a full remote
  desktop.
- App Store distribution, code signing, and notarization (roadmap).
- Mobile, web, and headless servers. With no graphical session there is
  nothing to badge.

---

## 2. Names and identifiers

| Thing | Value |
|---|---|
| Product name in UI and docs | **PAWPRINT** (all caps in UI chrome and headings; "PAWPRINT" or "Pawprint" in prose) |
| Binary, crate, config dir, release asset prefix | `pawprint` |
| Windows GUI-subsystem binary | `pawprintw.exe` ([11.1.6](#1116-two-executables-on-windows)) |
| Developer tool binary | `pawdev` (never shipped) |
| Repository | `https://github.com/nealhardesty/pawprint` |
| macOS LaunchAgent label | `io.github.nealhardesty.pawprint` |
| Wayland layer-shell namespace | `pawprint` |
| X11 `WM_CLASS` | `pawprint`, `Pawprint` |
| Windows window class | `PawprintBadge` |
| Environment variable prefix | `PAWPRINT_` |
| License | MIT, code and bundled art |

Mascot IDs are lowercase ASCII kebab-case, `[a-z0-9]+(-[a-z0-9]+)*`, at most
32 characters, and **stable forever** once released. Renaming a released ID
is a breaking change: it moves cached hosts and breaks user configs. Add an
alias instead.

---

## 3. Supported environments

### 3.1 Desktop targets

| Target | Backend | Window technology | 1.0 status |
|---|---|---|---|
| Windows 10 1809+ / 11, x86_64 | `windows` | Layered `WS_POPUP` window | **Supported** |
| macOS 13+, Apple Silicon and Intel (universal binary) | `macos` | Borderless non-activating `NSPanel` | **Supported** |
| Linux X11 (any EWMH window manager, with or without a compositor) | `x11` | ARGB or shaped override window | **Supported** |
| Linux Wayland, KDE Plasma 6 | `wayland` | `zwlr_layer_shell_v1` surface | **Supported** |
| Linux Wayland, GNOME 46+ (Ubuntu 24.04 LTS and newer) | `x11` with the `gnome` profile, via XWayland | ARGB override window ([11.5](#115-gnome-wayland-ubuntu)) | **Supported** |
| Linux Wayland, wlroots compositors (Sway, Hyprland, labwc, river, ...) | `wayland` | `zwlr_layer_shell_v1` surface | Expected to work, not in the acceptance matrix |
| Windows ARM64, Linux aarch64 | — | — | Linux aarch64 builds at 1.0; Windows ARM64 is on the roadmap |

### 3.2 Linux backend selection (`backend = "auto"`)

1. If `WAYLAND_DISPLAY` is set and the compositor advertises
   `zwlr_layer_shell_v1`, use `wayland`.
2. If `WAYLAND_DISPLAY` is set, `XDG_CURRENT_DESKTOP` contains `GNOME`, and
   `DISPLAY` is set, use `x11` with the **`gnome` profile** through XWayland
   ([11.5](#115-gnome-wayland-ubuntu)). If the Plan B presenter extension is
   on the session bus, use it instead.
3. Otherwise, if `DISPLAY` is set, use `x11`. This covers native X11
   sessions and xrdp and Xvnc sessions, which are X11 even when the console
   session is Wayland.
4. Otherwise exit with code 3 and a message pointing at `pawprint doctor`.

When step 3 runs inside a Wayland session that is neither Plasma nor GNOME,
PAWPRINT logs a warning, because always-on-top and click-through for
XWayland windows then depend on an untested compositor.

`backend = "x11"` or `"wayland"` in the config skips the detection (a
forced `x11` in a GNOME Wayland session still gets the `gnome` profile).
`"windows"` and `"macos"` mean nothing on Linux; they log a warning and act
like `auto`. The decision is a pure function (`backend::linux::select`),
unit-tested for every row above, and `pawprint doctor` prints it with its
reason.

### 3.3 Remote clients

PAWPRINT has no knowledge of remote clients. It draws into the desktop of the
session it runs in, and every client that streams that desktop shows it. What
matters is **which session** a client lands in:

| Client | Host OS | Session the client shows | Notes |
|---|---|---|---|
| RDP (mstsc, Windows App, Remmina, FreeRDP) | Windows | A separate RDP session, or the console session taken over | Each session has its own PAWPRINT instance, started by autostart at logon ([10.4](#104-single-instance-per-graphical-session)) |
| RDP | Linux (xrdp) | A separate X11 session | XDG autostart must run in that session. Minimal WMs may need a manual start. |
| RDP | Linux (KRdp, Plasma Wayland) | The live Plasma session, via screencast | Layer-shell overlays are part of the composited output **(verify)** |
| RDP, VNC | Linux (GNOME Remote Desktop, Ubuntu) | The console session (desktop sharing) or a headless GNOME session (remote login) | Mutter's screencast includes XWayland windows; see [11.5.2](#1152-remote-sessions-on-gnome) |
| VNC | macOS (Screen Sharing / ARD) | The console session | — |
| VNC | Linux (x11vnc, krfb, Xvnc) | Console (x11vnc, krfb) or its own X11 server (Xvnc) | Xvnc sessions often run without a compositor; see [11.3.2](#1132-transparency-with-or-without-a-compositor) |
| Jump Desktop, Parsec, Apollo/Moonlight | Windows, macOS | The console session, captured by screen capture | Virtual display drivers can make a virtual display primary; PAWPRINT follows display-change events |

---

## 4. The badge

### 4.1 Anatomy

Schematic only; exact art-pixel geometry is in [4.2](#42-geometry-art-pixels).

```
        ┌────────────┐
        │  (mascot)  │   mascot sprite, animated, 32×32 art pixels
     ┌──┴────────────┴──┐
     │▓▓▓▓ crate back ▓▓│   crate back rim (behind the mascot)
     │  ┌────────────┐  │
     │  │    OWL     │  │   crate front + label strip (in front of the mascot)
     │  └────────────┘  │
     └──────────────────┘
          ┌──────────────┐
          │ PROD-OWL-02  │   nameplate: used only when the label doesn't fit
          └──────────────┘
```

The badge is composed from three independent layers, back to front:

1. **Crate back:** shared art, tinted with the host's accent color.
2. **Mascot:** the selected mascot's current frame.
3. **Crate front with label strip:** shared art, tinted with the accent, with
   the label drawn on it.

The crate and label never move. Only the mascot animates. Because the crate
is separate from the mascot, any mascot can sit in any crate, and changing a
label never needs new art.

### 4.2 Geometry (art pixels)

All art is authored at **1× in art pixels** and scaled by an integer factor.

| Element | 1× geometry |
|---|---|
| Badge canvas (without nameplate) | 32 × 32 |
| Mascot frame | 32 × 32, drawn at (0, 0) |
| Crate back rim | rows 18–19 |
| Crate front | rows 20–31 (12 rows), full width; the mascot is hidden behind it |
| Label strip on crate front | rows 22–29, columns 2–29 (28 × 8 usable) |
| Outline | 1 art-pixel dark outline around the crate for contrast on any wallpaper |

Mascot art should keep its face and silhouette in rows 0–19 so it "peeks out"
of the crate. Anything drawn in rows 20–31 is covered by the crate front,
except where the crate front art is transparent.

### 4.3 Sizes and scaling

- `size` is the **mascot size in logical pixels**: `32`, `64` (default), or
  `128`. Any positive multiple of 32 up to 1024 is accepted (the cap keeps a
  typo from allocating a gigantic image).
- Art scale `s = size / 32`.
- Physical pixels per art pixel `P = max(1, round(s × display_scale))`, where
  `display_scale` is the OS scale factor of the target display (1.0, 1.25,
  1.5, 2.0, ...).
- Scaling is always **nearest-neighbor by an integer factor**, so pixel art
  stays crisp. At fractional display scales the badge is a little larger or
  smaller than the nominal size. That is intended.

### 4.4 Label

**Text.** Derived from the hostname ([5.2](#52-labels)) or configured.
Rendered with PAWPRINT's own embedded bitmap font: 5 × 7 glyphs on a 6-pixel
advance, covering printable ASCII (0x20–0x7E). Other characters render as `?`.
Hostnames are restricted to letters, digits, and hyphens, so they always
render cleanly. Labels longer than 20 characters are cut **in the middle**:
the first 9 and the last 9 characters, joined by `..`
(`jenkins-agent-linux-07` becomes `jenkins-a..-linux-07`), because the end of
a name is what tells one machine from the next.

**Why 20, and why the middle** (measured in M4 with `pawprint render` over a
corpus of default and fleet hostnames):

| Short name (examples) | Length | Badge at 32 / 64 / 128 |
|---|---|---|
| `nas`, `db02` | 3–4 | 32×32 / 64×64 / 128×128: on the crate |
| `web01`, `fedora`, `buildbox` | 5–8 | 67–85×32 / 70–106×90 / 128×128 (on the crate at 128 up to 9 characters) |
| `prod-owl-02`, `raspberrypi`, `k8s-master-1` | 10–12 | 97–109×32 / 130–154×90 / 130–154×154 |
| Windows defaults `DESKTOP-4F7Q2KL`, `WIN-3J8K2M4N5P6`; AWS `ip-172-31-45-201` | 15–16 | 127–133×32 / 190–202×90 / 190–202×154 |
| macOS defaults `Johns-MacBook-Air`, `MacBook-Pro-de-Maria`; `build-agent-linux-07` | 17–20 | 139–157×32 / 214–250×90 / 214–250×154 |
| `jenkins-agent-linux-07`, DigitalOcean `ubuntu-s-1vcpu-1gb-nyc1-01`, AKS/GKE nodes (33–35) | 22+ | cut to 20: 157×32 / 250×90 / 250×154 |

- Every default hostname scheme in the corpus (Windows, macOS, AWS, Raspberry
  Pi, typical fleet names) fits in 20, so 20 stays. A 20-character
  nameplate is 250 px wide at 64 and 128 (the font scale is 2 at both) and
  157 px beside a size-32 crate: noticeable but still a corner badge.
  A lower limit would cut the Windows and macOS defaults; a higher one
  makes the plate wider than most panels' clear corners.
- Only names of 22+ characters are cut, and for them the end matters most:
  cloud node names share a long prefix (`gke-prod-default-pool-…`) and differ
  in the suffix, and fleet names end in a number. Cutting the end (the
  provisional rule) made every node of a pool look the same. Fleets with
  such names are better served by an explicit `label` per `[hosts]` glob.
- Remote-client scaling: a client showing a 4K desktop in a 1080p window
  scales by 0.5. The default font scale 2 (size ≥ 64) then lands at one
  screen pixel per font pixel, the smallest legible size; the size-32 label
  (font scale 1) is not legible below about 0.75. Users of scaled clients
  should keep `size` at 64 or more or set `label_size = "large"`.

**Size.** `label_size` sets the font scale independently of the mascot:

| `label_size` | Font scale (logical px per font pixel) | Glyph height |
|---|---|---|
| `small` | 1 | 7 |
| `medium` | 2 | 14 |
| `large` | 3 | 21 |
| `auto` (default) | `medium` at size ≥ 64, `small` at size 32 | — |

**Placement.** Decided in this order:

1. **On the crate**, centered in the label strip, if the rendered text fits
   the strip at the current scale.
2. **Nameplate:** otherwise, a separate plate with the same accent and
   outline, attached under the crate and centered. It can be wider than the
   crate.
3. At `size = 32` with `label_size` `auto`, the nameplate goes **beside** the
   crate instead of under it, on the side facing the screen interior, so the
   badge grows inward from its anchored corner.

The label fits the strip when its width (`6n − 1` font pixels for `n`
characters) and height (7) times the font scale fit the strip's 28 × 8 art
pixels times the art scale. The nameplate is drawn in **font pixels**, so it
scales with `label_size`, not `size`: `plate-left`, then `plate-mid` once per
text column, then `plate-right` (all from `crate.toml`), with the text right
after `plate-left` and centered vertically. Under the crate, the plate is
centered horizontally; beside it, the plate's bottom lines up with the
crate's. The anchor/offset position applies to the whole badge, nameplate
included (`pawprint_core::layout::badge_layout`).

**Contrast.** Dark text (`#1b1b1f`) on a light strip (`#f2efe6`) with the
accent around it. The strip and text colors are fixed for legibility. Only
the accent changes.

**Case.** `label_case = "upper"` (default) renders the label like a shipping
stamp. `"as-is"` keeps the configured or detected case. `"lower"` lowercases.

### 4.5 Pixel rules

- Every color has alpha 0 or 255. No partial alpha in art or composition.
  This keeps edges crisp and makes the X11 shape fallback exact
  ([11.3.2](#1132-transparency-with-or-without-a-compositor)). It also makes
  premultiplied alpha trivial, which Windows and Wayland require.
- The renderer composes the badge into a small RGBA buffer at 1×, then
  scales it by `P` into the backend's buffer. A redraw happens only when the
  frame, label, size, or scale changes.

---

## 5. Host identity: hostname to label and mascot

### 5.1 Reading the hostname

| Platform | Source | Notes |
|---|---|---|
| Windows | `GetComputerNameExW(ComputerNameDnsHostname)` | Preserves case. Falls back to `ComputerNameNetBIOS` (uppercase, max 15 characters). |
| macOS | SystemConfiguration `HostName` if set, else `LocalHostName` | Avoid bare `gethostname()`: on macOS it can return a DHCP- or reverse-DNS-assigned name that changes between networks. `ComputerName` ("Someone's MacBook Pro") is a friendly name, not a hostname, and is never used. Read through the SystemConfiguration API (`objc2-system-configuration`): `HostName` from the `/System/System` preferences path, else `SCDynamicStoreCopyLocalHostName`, the same values `scutil --get` prints (checked in `make test-macos-display`). |
| Linux | `/proc/sys/kernel/hostname` (the kernel nodename, which is what `gethostname(2)` returns), else `/etc/hostname` | — |
| Any | `PAWPRINT_HOSTNAME` env var | Overrides detection. Used by tests and `pawprint explain --host`. |

### 5.2 Labels

1. `raw` = detected hostname, trimmed.
2. `short` = `raw` lowercased, cut at the first `.` (drops DNS suffixes and
   `.local`). `owl.lab.example` becomes `owl`, and `owl-02.lab.example`
   becomes `owl-02`.
3. Label by `label` setting:
   - `"auto"` (default): `short`
   - `"full"`: `raw` lowercased
   - `"none"`: no label, no strip text, no nameplate
   - any other string: used literally
4. Apply `label_case`, then truncate ([4.4](#44-label)).

### 5.3 Choosing a mascot

Rules apply in order. The first one that produces a mascot wins.
`pawprint explain` prints which rule fired.

| # | Rule | Example |
|---|---|---|
| 1 | **Explicit override.** `mascot = "<id>"` in config, from the matching `[hosts]` entry or the top level ([8.3](#83-precedence)). | `mascot = "cat-maine-coon-black"` |
| 2 | **Cached assignment.** A previous rule-5 result for this `short` name, read from the state file. | — |
| 3 | **Whole-name alias.** `squash(short)` equals a normalized alias. | `owl` → owl, `polar-bear` → polar-bear, `red-panda-01` → red-panda |
| 4 | **Token match.** Longest token n-gram that names exactly one mascot. | `prod-owl-02` → owl, `snowless` → none |
| 5 | **Deterministic fallback.** Rendezvous hash over all eligible mascots. The result is cached (rule 2). | `buildbox` → (stable mascot) |

**Normalization.**

- `squash(x)`: lowercase, then remove every character that is not `[a-z0-9]`.
  Aliases are compared in squashed form, so `polar-bear`, `polar_bear`, and
  `polarbear` are equivalent.
- `tokens(short)`: split on every non-alphanumeric character and at every
  letter↔digit boundary, then drop tokens that are entirely digits.
  `prod-owl02` → `[prod, owl]`.
- For rule 3, `squash(short)` is computed after removing all-digit tokens:
  `red-panda-01` → `redpanda`.

**Rule 4 in detail.** For `n` from `len(tokens)` down to 1, form every run of
`n` adjacent tokens, concatenated (`[prod, red, panda]` at `n = 2` gives
`prodred` and `redpanda`). Look each one up in the alias table. At the first
`n` that yields **any** match:

- exactly one distinct mascot → that mascot wins;
- more than one distinct mascot → no winner, fall through to rule 5 (ambiguity
  is resolved by the hash, never by token order).

Tokens are whole words only. `snowless` never matches `owl`, and `bearded`
never matches `bear`.

**Rule 5: rendezvous (HRW) hashing.**

```
score(host, id) = splitmix64( fnv1a64( utf8(short) ‖ 0x00 ‖ utf8(id) ) )
winner          = argmax over eligible ids of score; ties → lexicographically smallest id
```

- `fnv1a64`: offset basis `0xcbf29ce484222325`, prime `0x100000001b3`.
- `splitmix64(z)`: `z += 0x9e3779b97f4a7c15; z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9; z = (z ^ (z >> 27)) * 0x94d049bb133111eb; return z ^ (z >> 31)` (wrapping arithmetic).
- **Eligible** = every bundled and user mascot whose manifest does not set
  `fallback = false`.
- The function is implemented in-tree with no dependencies. **It must never
  change.** Fresh installs with the same catalog give a host the same answer
  everywhere, and that only holds while the function is frozen.

Test vectors (frozen, enforced by unit tests):

| Input bytes | `fnv1a64` | `splitmix64(fnv1a64)` |
|---|---|---|
| `""` | `0xcbf29ce484222325` | — |
| `"a"` | `0xaf63dc4c8601ec8c` | — |
| `"buildbox\0owl"` | `0x3251b145c0609532` | `0x959e9e0228cae98d` |
| `"buildbox\0otter"` | `0x46a1d27c6d1c72b2` | `0xc9611fca31e71b6f` |
| `"prod-db-01\0owl"` | `0x2412edb9d3386759` | `0xfa6c62cfbcb0e79d` |

**Why cache on top of HRW?** HRW already moves only about 1/(N+1) of hosts
when a mascot is added. Caching moves none: an established host keeps its
mascot through catalog growth. `pawprint explain --recompute` shows what the
hash would choose today, and `pawprint forget` clears this host's cache entry.

### 5.4 Accent color

| Mascot chosen by | Accent |
|---|---|
| Config `accent = "<name or #rrggbb>"` | The configured value |
| Rules 1, 3, 4 | The mascot manifest's `accent` |
| Rules 2, 5 (fallback) | `palette[ splitmix64(fnv1a64(utf8(short))) mod 16 ]` |

Fallback hosts that land on the same mascot are then still likely to differ
by crate color. The palette is in [Appendix B](#appendix-b-accent-palette).

---

## 6. Placement and move mode

### 6.1 Position model

A position is an **anchor** plus an **offset**, both measured inside the
**usable area** of the target display, meaning the display minus taskbars,
the menu bar, the Dock, and panels.

- `anchor`: one of `top-left`, `top`, `top-right`, `left`, `center`,
  `right`, `bottom-left`, `bottom`, `bottom-right`.
- `offset = [x, y]`: logical pixels from the anchor edge(s) **inward**. For
  `top-right`, `[16, 16]` means 16 px left of the right edge and 16 px below
  the top edge. Components along a centered axis shift from the center
  (positive = right/down).
- Default: `top-right`, `[16, 16]`.

Inside `pawprint-core`, every rectangle is in **physical pixels** in the
backend's desktop coordinate space. Offsets stay logical and are multiplied by
the target display's scale (rounded) when placed. Backends whose window APIs
work in logical units (macOS points, Wayland layer-shell margins) convert at
the edge.

The badge keeps its relationship to the nearest edges when the resolution
changes, which happens constantly with RDP reconnects and client window
resizes. If a position no longer fits, it is **clamped** into the usable
area. The saved value is not modified, so the badge returns to its spot when
the space comes back.

### 6.2 Target display

1. `display = "<name>"` if that output exists (names as reported by
   `pawprint doctor`).
2. The OS primary display.
3. The only display, if there is exactly one.
4. Otherwise the first display in the backend's enumeration order.

Display changes (hot-plug, resolution, scale, work area) trigger
re-evaluation and re-layout.

### 6.3 Move mode

The badge is click-through by default. To move it, the user enters
**move mode**:

- tray or menu-bar item **Move**, or
- `pawprint move` from any terminal in the same session.

In move mode:

1. The badge accepts the pointer and shows a 1-art-pixel dashed outline in the
   accent color: two art pixels on, two off, drawn over the edge of the badge
   image (`movemode::draw_outline`, by the core, so every backend shows it).
   The gaps keep the art underneath, so transparent corners stay transparent.
2. **Drag** it anywhere on the target display. It is constrained to the
   usable area.
3. **Release** to save. The new anchor is the ninth (3 × 3 grid) of the
   usable area containing the badge's center. The offset is recomputed from
   that anchor. Snapping to a corner keeps a corner position stable across
   resolution changes.
4. **Arrow keys** nudge by 1 logical px, **Shift+arrows** by 10, and
   **Enter** saves, where the platform gives the badge keyboard focus in move
   mode.
5. **Escape** cancels and restores the previous position.
6. Move mode **ends automatically after 30 seconds** without interaction,
   saving any completed drag.

After move mode the badge is click-through again. `pawprint reset-position`
deletes the saved position and returns to the configured or default one.

### 6.4 Where positions are stored

A dragged position is written to the **state file**
([8.1](#81-files-and-locations)), not the config file, so PAWPRINT never
rewrites a hand-edited config. Effective position = state file position, if
present, else config `position`/`offset`, else default. `pawprint explain`
shows which one is in effect.

---

## 7. Animation and power

### 7.1 Model

- Each mascot has a **rest frame** and one or more **actions** (sequences of
  frames with per-frame durations), defined in its source
  ([12.3](#123-mascot-source-format)).
- The badge shows the rest frame. Every `interval` seconds (uniformly random
  in `[8, 20]` by default) it plays one action, chosen by weight among the
  actions whose `when` matches the local time of day, then returns to rest.
  Rare actions are simply low-weight actions ([12.5](#125-animation-extras-that-cost-nothing-at-runtime)).
- Frame durations default to 200 ms (5 FPS). Allowed range: 80–2000 ms.
  An action lasts at most 4 seconds.
- `animation = "static"` shows the rest frame forever and schedules nothing.
- The random source is a small in-tree PRNG (xorshift64*) seeded from the
  clock at startup. Animation timing does not need to be reproducible.

### 7.2 Power budget

| Metric | Budget |
|---|---|
| Wakeups while resting | **Zero** periodic wakeups. The only timer is the deadline for the next action. |
| CPU, idle mode, 10-minute average | < 0.1% of one core |
| CPU, static mode | 0% outside display or config events |
| Resident memory | < 30 MB |
| Redraws | Only on a frame, label, size, scale, or position change |
| Startup to visible | < 300 ms |
| Binary size, all 100 mascots embedded | < 15 MB |

The event loop blocks on OS events with a timeout equal to the next animation
deadline. There is no fixed tick.

---

## 8. Configuration

### 8.1 Files and locations

| Purpose | Linux | macOS | Windows |
|---|---|---|---|
| Config (`config.toml`) | `$XDG_CONFIG_HOME/pawprint/` (`~/.config/pawprint/`) | `~/Library/Application Support/pawprint/` | `%APPDATA%\pawprint\` |
| User mascots (`mascots/<id>/`) | config dir | config dir | config dir |
| State (`state.toml`) | `$XDG_STATE_HOME/pawprint/` (`~/.local/state/pawprint/`) | `~/Library/Application Support/pawprint/` | `%LOCALAPPDATA%\pawprint\` |
| Log (`pawprint.log`, max 1 MB, one rotation) | state dir | `~/Library/Logs/pawprint/` | state dir |
| IPC endpoint | `$XDG_RUNTIME_DIR/pawprint/<session>.sock` (`/tmp/pawprint-<uid>/` without `XDG_RUNTIME_DIR`) | `$TMPDIR/pawprint/<session>.sock` | `\\.\pipe\pawprint-<user>-<session-id>` |
| Installed binary | `~/.local/bin/pawprint` | `~/.local/bin/pawprint` | `%LOCALAPPDATA%\Programs\pawprint\` |

`pawprint paths` prints all of these for the current machine. The config file
is optional and is never created automatically. `pawprint config init` writes
a fully commented one.

### 8.2 `config.toml` reference

Every key is optional. The values shown are the defaults.

```toml
# Which mascot to show. "auto" picks one from the hostname (see `pawprint explain`).
mascot = "auto"            # "auto" | a mascot id from `pawprint list`

# Text on the crate.
label = "auto"             # "auto" (short hostname) | "full" | "none" | any text
label_case = "upper"       # "upper" | "lower" | "as-is"
label_size = "auto"        # "auto" | "small" | "medium" | "large"

# Mascot size in logical pixels. 32, 64, and 128 are the presets; any multiple of 32 works.
size = 64

# Where the badge sits on the target display. Dragging in move mode overrides this.
position = "top-right"     # top-left | top | top-right | left | center | right | bottom-left | bottom | bottom-right
offset = [16, 16]          # logical px inward from the anchor
display = "primary"        # "primary" | an output name from `pawprint doctor`

# Animation.
animation = "idle"         # "idle" | "static"
interval = [8, 20]         # seconds between idle actions (min, max)

# Crate color.
accent = "auto"            # "auto" | a palette name (see `pawprint list --accents`) | "#rrggbb"

# Tray icon (Windows), menu-bar item (macOS), or StatusNotifierItem (Linux).
tray = true

# Window backend. Leave on auto unless `pawprint doctor` says otherwise.
backend = "auto"           # "auto" | "windows" | "macos" | "x11" | "wayland"

# Per-host overrides. Keys are short hostnames or glob patterns (* and ?).
# Any top-level key may appear in a host table.
[hosts.buildbox]
mascot = "capybara"

[hosts."prod-*"]
accent = "cranberry"
label_case = "upper"
```

Unknown keys are a **warning**, not an error, so a newer fleet config still
loads on older binaries. Invalid values (bad mascot id, bad accent) log a
warning and fall back to the default for that key. A config file never
prevents the badge from starting.

### 8.3 Precedence

For each key, highest wins:

1. Command-line flags on `pawprint run` (debugging only)
2. `[hosts."<short>"]`: exact match on the short hostname
3. `[hosts."<glob>"]`: matching globs, most specific first (longest pattern,
   then fewest wildcards, then file order)
4. Top-level keys
5. Built-in defaults

The same `config.toml` can be copied to every machine in a fleet. Each host
picks up its own section.

### 8.4 State file

`state.toml` is owned by PAWPRINT and rewritten atomically (write to temp,
then rename).

```toml
version = 1

[position]                 # present only after a drag
anchor = "top-right"
offset = [16, 16]

[assignments]              # rule-5 cache: short hostname → mascot id
buildbox = "capybara"
```

### 8.5 Reloading

The running badge reloads config when `pawprint reload` is run, when the tray
item **Reload config** is chosen, and when the config file's modification
time changes. The last is checked only on display or IPC events and on the
next animation deadline, so there is no file-watch thread: `Shell::tick`,
which runs after every wakeup of the loop, compares one `stat` with the
modification time recorded at the last read. A static badge with no events
therefore notices an edit at its next event (`pawprint status` is one).

### 8.6 Editing the config: `set`, `unset`, and the tray

`pawprint set <key> <value>`, `pawprint unset <key>`, and the tray's **Size**
and **Animation** choices all edit the config the same way
(`pawprint_core::config_edit`):

- **Where:** the exact `[hosts."<short>"]` table for this host's short name
  (an existing table that matches it case-insensitively, else a new one,
  appended at the end). An exact host table outranks globs and top-level
  keys (8.3), so the choice always takes effect, and a config copied to
  another machine doesn't carry it over. Only `pawprint run` flags rank
  higher.
- **How:** with `toml_edit`, so comments, order, and formatting stay. A
  value already in the table keeps its trailing comment. `unset` removes
  the key, then the table if it is left empty without comments, then an
  empty implicit `[hosts]`. A file that isn't valid TOML is left alone and
  the command fails. The write is atomic (temporary file, then rename) and
  goes to a symlink's target, so dotfile managers keep their link.
- **Values** are read as TOML (`64`, `false`, `[8, 20]`, `"quoted"`) and
  otherwise as a plain string (`otter`), then checked by the parser that
  loads the file (`config::check_value`). `set` also resolves the host
  with the new file and refuses values that would only produce a warning
  (a mascot or accent that doesn't exist), before writing anything.
- **Then:** `set` and `unset` send `reload` to this session's badge if one
  runs. The tray writes, then reloads. If the file can't be written, a tray
  choice still applies until the badge restarts (as a session-only
  override) and the log says why.

`pawprint config init` writes the commented template
(`config_edit::TEMPLATE`, 8.2 with the host examples commented out and an
empty `[hosts]` at the end, so host tables land after the examples) and
never overwrites a file.

---

## 9. Command-line interface

One binary, subcommands via `clap`. Every command that talks to a running
badge reaches the instance **in the current graphical session**.

| Command | What it does |
|---|---|
| `pawprint` | Start the badge in the background for this session and return. If it is already running, say so and exit 0. It waits up to 5 s for the new badge to answer; if the badge exits first, `pawprint` exits with the badge's code and points at the log and `doctor`. |
| `pawprint run` | Run the badge in the foreground. Autostart entries use this. `--verbose` logs to stderr. |
| `pawprint move` | Enter move mode on the running badge. |
| `pawprint reset-position` | Forget the dragged position. |
| `pawprint reload` | Re-read `config.toml`. |
| `pawprint quit` | Stop the running badge. |
| `pawprint status` | Is it running? Backend, display, mascot, label, position. |
| `pawprint explain [--host NAME] [--recompute]` | Show the hostname pipeline, each mascot rule's result, which one won and why, and the effective settings with their source. Works without a running badge. |
| `pawprint list [--category C] [--accents]` | List mascots (id, name, category, aliases, source: bundled/user/override), or the accent palette. |
| `pawprint preview <id> [--size N]` | Open a temporary, normal (focusable, closable) window showing that mascot cycling through all its actions (every action in file order, `when` ignored, 1 s apart), on a slate backdrop with a 16 px margin, centered, at size 128 by default. Esc or closing the window ends it. It uses this host's label and accent and the badge's own backend surface in preview mode; it writes nothing. |
| `pawprint render <id> --out FILE.png [--size N] [--label TEXT] [--frame N]` | Write a badge image. Used for docs and golden tests. |
| `pawprint set <key> <value>` | Write `<key>` into this host's `[hosts."<short>"]` table using `toml_edit` (keeps comments and formatting), then reload ([8.6](#86-editing-the-config-set-unset-and-the-tray)). Example: `pawprint set mascot otter`. Exit code 2 for an unknown key or invalid value. |
| `pawprint unset <key>` | Remove that key from this host's table, then reload. |
| `pawprint forget` | Drop this host's cached fallback assignment (through the running badge if there is one, which picks again and saves; else in the state file) and say what the hash picks now. A host whose mascot comes from another rule has nothing to forget. |
| `pawprint config init` | Write a commented `config.toml` if none exists. |
| `pawprint config path` | Print the config path. |
| `pawprint mascot export <id> <dir>` | Write a bundled mascot to `<dir>` in the simple PNG form ([12.7](#127-png-bridges)) as a starting point for custom art. |
| `pawprint mascot check <dir>` | Validate a custom mascot directory with the same rules as the build ([12.11](#1211-validation-rules)). |
| `pawprint autostart on\|off\|status` | Register or remove launch at graphical login for this user: an entry that runs `<this executable> run` (on Windows `pawprintw.exe run`). Linux: `$XDG_CONFIG_HOME/autostart/pawprint.desktop`. Windows and macOS: [11.1.5](#1115-tray-and-autostart), [11.2](#112-macos). |
| `pawprint doctor` | Diagnose the environment: session type, chosen backend and why, compositor/WM, layer-shell, ARGB visual, compositing manager, displays with names, scales and work areas, primary detection, GNOME fractional-scaling blur (with the fix), tray availability, IPC endpoint, autostart state, config warnings. |
| `pawprint paths` | Print every file location ([8.1](#81-files-and-locations)). |
| `pawprint gnome-extension install\|remove` | Plan B only ([11.5.3](#1153-plan-b-the-presenter-extension)): install or remove the GNOME presenter extension. |
| `pawprint --version` | Version, git commit, target triple, catalog size. |

Exit codes: `0` ok, `1` error, `2` usage error, `3` no usable graphical
session or backend, `4` no running badge (for commands that need one).

---

## 10. Runtime architecture

### 10.1 Shape

```
┌──────────────────────────── pawprint (binary) ─────────────────────────────┐
│  cli ──► ipc client ─────────────────┐                                     │
│                                      ▼                                     │
│  run ──► backend (one of 4) ◄──► pawprint-core::Badge  (pure state machine)│
│            │  native event loop      │ start(displays) -> Vec<Effect>     │
│            │  window / surface       │ handle(Event) -> Vec<Effect>       │
│            │  tray                   │ next_deadline() -> Option<Instant> │
│            └─ ipc server thread ─────┘                                     │
└────────────────────────────────────────────────────────────────────────────┘
pawprint-core: hostname rules · matching · HRW · config · state · catalog ·
               layout · animation scheduler · compositor · font   (no OS deps)
```

**Functional core, imperative shell.** `pawprint-core` contains every
decision and has no platform dependencies, so it is fully unit-testable on
any OS. Each backend is a thin adapter that:

1. owns the native event loop and window or surface;
2. translates native events into `core::badge::Event`: `Deadline`,
   `DisplaysChanged(Vec<DisplayInfo>)`, `DragStart/DragMove/DragEnd(Point)`,
   `Key(Key)`, `Command(Command)` (from IPC or the tray menu: `Move`,
   `ResetPosition`, `Reload`, `Quit`, `SetSize(n)`, `SetAnimation(mode)`,
   `Forget`),
   and `ConfigTouched(Config)` (the shell noticed a change and re-read the
   file);
3. executes `core::badge::Effect`s: `Present(Frame)` (already scaled),
   `Place(PhysicalRect)`, `SetInteractive(bool)`, `SetTray(TrayState)`,
   `SaveState(State)` (write it atomically), `ReloadConfig` (re-read the file,
   then send `ConfigTouched`), and `Exit`;
4. blocks until the next native event or `core.next_deadline()`.

Driving the core, in full (every backend's loop has this shape):

```rust
let inputs = Inputs { hostname, config, state, flags, seed: clock_seed };
let mut badge = Badge::new(Catalog::bundled(), catalog::bundle(), inputs, Instant::now());
let mut effects = badge.start(surface.displays(), now());
loop {
    for effect in surface::apply(&mut surface, effects) {   // Present/Place/SetInteractive
        match effect { SetTray(t) => .., SaveState(s) => s.save_atomic(..), ReloadConfig => .., Exit => return, _ => {} }
    }
    let event = wait_for_native_event_or(badge.next_deadline());   // Deadline on timeout
    effects = badge.handle(event, now());   // now() = Now { instant, local_hour }
}
```

The core emits `Place` and `Present` only when the rectangle or the pixels
actually change, so animation frames never re-place the window and a resting
badge never redraws. The local hour is an input (`Now::local_hour`) because
reading the time zone is a platform call.

The IPC server runs on its own thread and posts commands into the native loop
(`PostMessageW` on Windows, the main dispatch queue on macOS, a `calloop`
channel on Linux). Every request, `status` included, is answered by the
loop, so a badge that answers is a badge whose loop is alive.

In code, everything in that loop except the native wait is written once,
in `pawprint::shell::Shell`:

| Call | When the backend makes it |
|---|---|
| `Shell::start(&mut surface)` | Once, when the surface can report its displays |
| `Shell::handle(&mut surface, event)` | For each translated native event. Events before `start` and repeated identical display sets are dropped, so a backend reports every native change without filtering. |
| `Shell::request(&mut surface, incoming)` | For each IPC request the loop receives |
| `Shell::timeout()` | Before sleeping: how long the native wait may last (`None` = until an event) |
| `Shell::tick(&mut surface)` | After every wakeup; sends `Deadline` once it has passed |
| `Shell::done()` | After every wakeup; true after `Quit` |
| `Shell::set_tray(Box<dyn Tray>)` | Once, to attach the platform's tray; the shell calls `Tray::show(&TrayState)` with every `SetTray` (also at once, if the core already described it) |
| `Shell::menu(&mut surface, command)` | For each tray menu choice. Size and Animation are saved first (8.6), then everything is the same `Command` IPC sends |
| `Shell::is_preview()` | When creating the surface: true for `pawprint preview` (a normal window) |

The shell runs the surface effects through `surface::apply`, writes
`SaveState` atomically, re-reads the config on `ReloadConfig` (and when its
modification time changes, 8.5), hands `SetTray` to the tray, logs, and
builds the `status` answer. In preview mode it wraps the surface in an
adapter that draws each frame on the backdrop with the margin and grows the
window to match, writes no state, and has no IPC or tray, so a backend's
preview is its own surface with a flag. On Linux, `backend::linux::drive` is the whole
loop for both backends.

### 10.2 Why not winit, Tauri, or Qt

PAWPRINT needs window types that general toolkits don't expose: a
non-activating `NSPanel` at status level that joins all Spaces, a layered
Win32 window with `WS_EX_TRANSPARENT` toggled at runtime, X11 input shapes,
and Wayland layer-shell. `winit` has no layer-shell support, and its
`WindowLevel` is unsupported on Wayland. Tauri and Qt bring a webview or a
large C++ runtime for a 64-pixel image. Direct platform APIs come to four small backends (Windows, macOS, X11, Wayland; GNOME reuses X11) over a shared core ([10.6](#106-code-sharing-dry)), and give the smallest binary and the lowest idle cost.

### 10.3 Key dependencies

| Purpose | Crate |
|---|---|
| CLI | `clap` (derive) |
| Config | `serde`, `toml`, `toml_edit` (for `pawprint set`) |
| Paths | `directories` |
| PNG decode/encode | `png` |
| Sprite compiler | `pawprint-sprite` (in-tree crate) |
| Embedded asset bundle codec | `postcard` (serde, `alloc` only) |
| Windows | `windows-sys` (raw Win32 bindings, no COM wrappers needed): window, GDI, DPI, named pipes, session id, hostname |
| macOS | `objc2`, `objc2-foundation`, `objc2-app-kit` |
| X11 | `x11rb` 0.14 (pure Rust; features `shape`, `xfixes`, `randr`) |
| Wayland | `smithay-client-toolkit` 0.21 (`default-features = false, features = ["calloop"]`: the default `xkbcommon` feature links libxkbcommon), `wayland-client` 0.31 (no features: `system`/`dlopen` load libwayland and `log` compiles a C shim), `wayland-protocols`, `wayland-protocols-wlr`, `wayland-protocols-plasma`, `calloop` 0.14 |
| Linux tray | `ksni` 0.3 (`default-features = false, features = ["blocking", "async-io"]`, which avoids tokio) over `zbus` 5 (`default-features = false, features = ["async-io", "blocking-api"]`; ksni 0.3 needs zbus 5, and zbus 6 has no `blocking` module); `zbus` also carries the Plan B presenter protocol. `blocking` is pinned to 1.6 (F36) |
| Logging | `log` + a minimal file logger (`pawprint::logger`: 1 MB cap, one rotation to `pawprint.log.1`; dependencies log at warn and above only, since zbus logs every D-Bus call at info) |
| IPC encoding | `serde_json` |
| Unix calls | `libc`: `localtime_r` (the local hour, `Now::local_hour`), `setsid` (detached start), `getuid`. Windows uses `GetLocalTime` from `windows-sys`. |

No hashing, PRNG, font, or image-scaling crates: those are small, frozen,
and in-tree.

### 10.4 Single instance per graphical session

A console session and an RDP session on the same machine are different
desktops, and each needs its own badge. The IPC endpoint doubles as the
instance lock, and it is keyed by **graphical session**, not by user:

| Platform | Session key | Endpoint |
|---|---|---|
| Windows | `ProcessIdToSessionId` | Named pipe `\\.\pipe\pawprint-<user>-<sid>` |
| macOS | GUI security session (`SessionGetInfo` audit session id; a session without window server access, such as SSH, has no key) | Unix socket `$TMPDIR/pawprint/asid-<id>.sock` (`TMPDIR` unset: the per-user temporary directory from `confstr(_CS_DARWIN_USER_TEMP_DIR)`) |
| Linux Wayland | `WAYLAND_DISPLAY` | `$XDG_RUNTIME_DIR/pawprint/wayland-<name>.sock` |
| Linux X11 | `DISPLAY` | `$XDG_RUNTIME_DIR/pawprint/x11-<display>.sock` |

On Linux the key comes from the environment alone, `WAYLAND_DISPLAY` first,
so a terminal in the session finds its badge without probing any display
server. GNOME on Wayland keys on `WAYLAND_DISPLAY` even though the badge
draws through XWayland. The X11 key keeps host and display number and drops
the screen (`:0`, `:0.0`, and `unix:0` are all `x11-0`; `localhost:10.0` is
`x11-localhost_10`). An absolute `WAYLAND_DISPLAY` has its `/` replaced.

Startup (`instance::claim`): bind the endpoint. If it already exists and a
live instance answers `status`, print "already running" and exit 0. If
nobody answers, it is stale (a crashed badge): remove it and bind again.
Binding first means two badges started at once can't both win. Sockets are
created mode `0600` in a `0700` directory that must belong to the user. A
clean exit removes the socket.

The session key and the transport are the only platform parts:
`instance::session_key()` per OS, and `ipc::Transport` (`bind`, `accept`,
`connect`, `remove_stale`), with `ipc::unix::UnixSocket` for Linux and
macOS and `ipc::windows::NamedPipe` for Windows. `bind` must fail with
`AddrInUse` while the endpoint exists; that is what makes it a lock.

The Windows key is `<user>-<session id>` (`GetUserNameW`, with `\` replaced
by `_`). The first pipe instance is created with
`FILE_FLAG_FIRST_PIPE_INSTANCE`, which fails with "access denied" when the
name exists; that maps to `AddrInUse`. The server connects one instance at a
time and creates the next before handing a connected one to the IPC thread,
so the name exists for as long as the badge runs and vanishes when it dies
(`remove_stale` has nothing to do). The pipe has the default DACL: full
control for the creating user, SYSTEM, and administrators, read-only for
everyone else, so only they can write a request; `PIPE_REJECT_REMOTE_CLIENTS`
refuses network clients. Pipe reads have no timeout (only those users can
connect).

### 10.5 IPC protocol

Newline-delimited JSON, one request and one response per connection:

```json
{"v":1,"cmd":"move"}
{"v":1,"ok":true}
{"v":1,"ok":false,"error":"no target display"}
```

Commands: `status`, `move`, `reset-position`, `reload`, `quit`, `forget`. `status`
returns the same fields `pawprint status` prints, under `"status"`: `pid`,
`backend`, `session`, `display`, `rect` (`[x, y, w, h]`, physical),
`scale`, `mascot`, `label`, `position`, `position_source`, `size`,
`moving`. `move` answers `no target display` when there is nothing to move.
`quit` answers before the badge exits. Unknown `v`, unknown commands, and
malformed lines get an error response. Either side gives up after 5
seconds, and a request line is at most 4 KiB. The CLI and the badge always
come from the same binary, so the protocol carries no compatibility shims
beyond the version check.

### 10.6 Code sharing (DRY)

Platform code is unavoidable. Duplicated platform code is not. The rules:

1. **Backends translate, they never decide.** A backend turns native events
   into `core::Event`s and executes `core::Effect`s. Any `if` about layout,
   snapping, timing, labels, mascots, or config belongs in `pawprint-core`.
2. **Logic that appears in two backends moves to the core** in the change
   that introduces the second copy.
3. **Each backend stays small,** around 500–800 lines including its tray and
   autostart glue. A backend growing past that is a sign that rule 1 is being
   broken.
4. **The core has a `FakeSurface`,** so move mode, display changes, reloads,
   and animation are tested in `make test` on every OS, without a window.

The backend surface is deliberately narrow:

```rust
pub trait Surface {
    fn displays(&self) -> Vec<DisplayInfo>;        // geometry, usable area, scale, primary, name
    fn present(&mut self, frame: &Frame);          // premultiplied BGRA, already scaled
    fn place(&mut self, rect: PhysicalRect);
    fn set_interactive(&mut self, on: bool);       // move mode on/off
}
```

`surface::apply(&mut surface, effects)` runs the three surface effects and
returns the rest to the shell, so no backend re-implements effect dispatch.
`surface::FakeSurface` records presents, places, and the interactive flag;
`badge.rs`'s tests drive startup, animation, display changes, move mode,
tray commands, and config reloads through it.

What is shared, and by whom:

| Shared piece | Lives in | Used by |
|---|---|---|
| Hostname rules, matching, HRW, config, state, catalog | `pawprint-core` | All backends, CLI |
| Layout, clamping, 3 × 3 snap, nameplate placement | `pawprint-core::layout` | All backends |
| Move mode: drag math, constraint, nudge, Esc, 30 s timeout | `pawprint-core::movemode` | All backends report raw pointer/key events only |
| Animation scheduler, `next_deadline()` | `pawprint-core::anim` | All backends |
| Compositor, integer scaler, font | `pawprint-core::render` | All backends, `render`, `preview`, golden tests |
| **One pixel format**: premultiplied little-endian ARGB8888 (bytes B, G, R, A) plus a 1-bit mask | `pawprint-core::render` | Windows `UpdateLayeredWindow`, X11 32-bit visual, `wl_shm` `ARGB8888`, macOS `CGImage`, presenter extension (F30); the mask feeds the X11 shape fallback |
| Tray menu model (items, checks, actions) | `pawprint-core::menu` | Windows tray, macOS menu bar, `ksni` |
| Sprite compiler | `pawprint-sprite` crate | `build.rs`, runtime user mascots, `pawdev`, `pawprint mascot check` |
| IPC protocol and server loop | `pawprint::ipc` | Unix-socket transport shared by Linux and macOS; named-pipe transport on Windows |
| Autostart interface (`on`/`off`/`status`) | `pawprint::autostart` | Three small implementations behind one interface (Linux XDG done; Windows and macOS are hooks returning "not implemented on this platform yet") |
| Config edits (`set`, `unset`, tray choices, `config init`) | `pawprint-core::config_edit` | CLI, `Shell::menu` |
| X11 backend | `pawprint::backend::x11` | Native X11 **and** GNOME via XWayland (profile flag) |
| `calloop` event loop, Linux scale helpers | `pawprint::backend::linux` | X11, Wayland, GNOME |
| StatusNotifierItem tray (`ksni`) | `pawprint::backend::sni` | X11, Wayland, GNOME |
| Detach / single-instance startup | `pawprint::instance` | All platforms (only the session-key function differs) |
| The shell around `Badge`: effects, state saving, reloads, status, log | `pawprint::shell` | All backends |
| Doctor report and the shared checks | `pawprint::doctor` | All platforms; each backend adds its own section |

---

## 11. Platform backends

Each backend must deliver the same **contract**, using the shared pieces in
[10.6](#106-code-sharing-dry):

1. Transparent everywhere the badge is transparent.
2. Above normal application windows.
3. Click-through outside move mode, with no focus stealing ever.
4. Hidden from the taskbar, Dock, Alt-Tab, and pagers.
5. Visible on every virtual desktop or Space.
6. Re-layout on display, scale, and work-area changes.
7. Move mode as specified in [6.3](#63-move-mode).
8. Captured by screen-capture-based remote tools (never opt out of capture).

### 11.1 Windows

#### 11.1.1 Window

- Class `PawprintBadge`, style `WS_POPUP`.
- Extended style `WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST |
  WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE`.
  - `WS_EX_LAYERED` + `UpdateLayeredWindow` with a 32-bit **premultiplied
    BGRA** DIB section and `ULW_ALPHA` / `AC_SRC_ALPHA` gives per-pixel
    transparency.
  - `WS_EX_TRANSPARENT` (with layered) makes the window click-through.
  - `WS_EX_TOOLWINDOW` keeps it out of the taskbar and Alt-Tab.
  - `WS_EX_NOACTIVATE` prevents activation. Also handle `WM_MOUSEACTIVATE` →
    `MA_NOACTIVATE`.
- Show with `ShowWindow(SW_SHOWNOACTIVATE)`, once the window has both a
  place and a frame. `Place` is `SetWindowPos(HWND_TOPMOST, ..,
  SWP_NOACTIVATE)`, which re-asserts topmost every time; `Present` copies the
  shared frame into the DIB section and calls `UpdateLayeredWindow` with no
  destination point, so it never moves the window.
- The loop (`backend::windows::run`): drain the message queue, hand the
  events `wndproc` recorded and the queued IPC requests to the shell, `tick`,
  then `MsgWaitForMultipleObjects(QS_ALLINPUT)` with `Shell::timeout()`. The
  IPC thread queues each request on a channel and wakes the loop with
  `PostMessageW(WM_APP)`. `wndproc` only records core events in a
  thread-local queue: Win32 calls such as `SetWindowPos` send messages
  synchronously, and this keeps the shell from being re-entered. `WM_CLOSE`
  and `WM_QUIT` quit cleanly.
- Never call `SetWindowDisplayAffinity`. `WDA_EXCLUDEFROMCAPTURE` would hide
  the badge from Parsec, Apollo, and Jump.

#### 11.1.2 DPI

Call `SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2)`
first thing in `run`. Handle `WM_DPICHANGED`.

#### 11.1.3 Displays

`EnumDisplayMonitors` + `GetMonitorInfoW` (`MONITORINFOF_PRIMARY`, `rcWork`
for the usable area). Re-layout on `WM_DISPLAYCHANGE`, `WM_DPICHANGED`, and
`WM_SETTINGCHANGE` with `SPI_SETWORKAREA`. Re-assert `HWND_TOPMOST` after each
display change. The scale is `GetDpiForMonitor(MDT_EFFECTIVE_DPI)` ÷ 96. A
display's name is its GDI device name without the `\\.\` prefix
(`DISPLAY1`), which is what `display = "..."` matches and `doctor` lists.
`WM_DPICHANGED`'s suggested rectangle is ignored: the core places the badge.

#### 11.1.4 Move mode

Clear `WS_EX_TRANSPARENT` so the badge takes the pointer, and report the drag
to the core like every other backend (no `HTCAPTION` native drag, which would
move the window outside the core's constraint, snap, and save):

- `WM_LBUTTONDOWN` → `SetCapture`, `DragStart`; `WM_MOUSEMOVE` while
  captured → `DragMove`; `WM_LBUTTONUP` → `DragEnd`, `ReleaseCapture`. Losing
  the capture (`WM_CAPTURECHANGED`, e.g. Alt+Tab) also ends the drag.
- Positions come from `GetCursorPos`, which is in physical desktop pixels
  under per-monitor v2, not from `lParam`: the window moves under the
  pointer, so window-relative coordinates would run away (compare F7).
- The core constrains, snaps, and saves, and answers with `Place`.
- Hit testing is per pixel: fully transparent pixels pass the pointer
  through even in move mode, so the drag starts on the mascot or the crate.
  The class cursor is the four-way move arrow, shown only in move mode.
- Keys: `WM_KEYDOWN` arrows, Enter, and Escape, with Shift from
  `GetKeyState`. The keyboard needs the foreground, which Windows gives a
  background process only with permission: `pawprint move` calls
  `AllowSetForegroundWindow(ASFW_ANY)` first (it runs in the foreground
  terminal), and the badge calls `SetForegroundWindow` on entering move mode
  and hands the foreground back to the previous window when it ends. Where
  Windows refuses, dragging still works. A click never activates the badge
  (`WS_EX_NOACTIVATE`, `MA_NOACTIVATE`).

#### 11.1.5 Tray and autostart

*M4, to do.* The shared half is done: implement `shell::Tray` (its `show`
gets a `TrayState`: `visible`, `tooltip`, the menu model, and `icon`, the
32 × 32 straight-RGBA rest frame in its crate), attach it with
`Shell::set_tray`, and hand each menu choice to `Shell::menu(&mut window,
command)` from the loop (never from inside `wndproc`, like every other
recorded event). The tray's callback message goes next to `WM_APP` in
`wndproc`, and `TaskbarCreated` re-adds the icon after Explorer restarts.
Autostart fills the hook in `pawprint::autostart`.

- Tray: `Shell_NotifyIconW` with the mascot's rest frame as a 16/32-pixel
  icon. Menu: **Move**, **Size ▸ 32/64/128**, **Animation ▸ Idle/Static**,
  **Reset position**, **Reload config**, **Quit**.
- Autostart: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value
  `pawprint` = `"<install dir>\pawprintw.exe" run`. HKCU Run entries run at
  every interactive logon, RDP logons included.

#### 11.1.6 Two executables on Windows

A GUI-subsystem executable has no attached console, so CLI output is lost
and the shell does not wait for it. A console-subsystem executable flashes a
console window when started at logon. PAWPRINT therefore ships **both** from
the same code (two `[[bin]]` targets with one shared `main`):

- `pawprint.exe` (console subsystem): the CLI. Bare `pawprint` and
  `pawprint run --detach` spawn `pawprintw.exe run` with `DETACHED_PROCESS`.
- `pawprintw.exe` (`#![windows_subsystem = "windows"]`): the badge, and the
  autostart target.

The code is the `pawprint` library crate (`src/lib.rs`, `pawprint::main`);
`src/main.rs` and `src/bin/pawprintw.rs` are three-line wrappers. On other
systems `pawprintw` builds as a stub that points at `pawprint`. Bare
`pawprint` starts the `pawprintw.exe` next to it, falling back to itself,
after marking its own stdio handles non-inheritable (FW8). A GUI-subsystem process still writes to stdout and stderr handles it
inherits, so `pawprintw run --verbose` with redirected output logs to that
file.

#### 11.1.7 Known limits

- Exclusive-fullscreen games and the secure desktop (UAC, lock screen) cover
  the badge. This is expected.
- Windows virtual desktops: whether an unowned topmost tool window appears
  on all virtual desktops needs checking **(verify)**. If it doesn't,
  re-show on `IVirtualDesktopManager` desktop switch, or accept it as a
  limitation.

### 11.2 macOS

Implemented in `backend::macos` with `objc2`, `objc2-foundation`,
`objc2-app-kit`, `objc2-quartz-core`, `objc2-core-graphics`, and `dispatch2`.

- `NSApplication` with activation policy **Accessory**, set at runtime, so
  there is no Dock icon and no app menu. No `.app` bundle is needed.
- `NSPanel` (a subclass whose only override is `canBecomeKeyWindow = YES`,
  since borderless windows otherwise refuse key status), style mask
  `borderless | nonactivatingPanel`, `isOpaque = false`,
  `backgroundColor = clear`, `hasShadow = false`,
  `ignoresMouseEvents = true`, `level = .statusBar` (window layer 25),
  `collectionBehavior = canJoinAllSpaces | stationary | fullScreenAuxiliary
  | ignoresCycle`, `hidesOnDeactivate = false`. It is ordered in with
  `orderFrontRegardless` at the first `Place`, never made key or main
  outside move mode.
- Content: a layer-**hosting** `NSView` (our own `CALayer`, set before
  `wantsLayer`) whose `contents` is a `CGImage` of the scaled badge, with
  `magnificationFilter = nearest`. The frame's premultiplied BGRA bytes are
  `kCGImageAlphaPremultipliedFirst | kCGImageByteOrder32Little` (F30),
  tagged with the panel screen's color space so the pixels reach the screen
  unconverted, as on Windows and Linux. Contents and frame changes run with
  `CATransaction.disableActions`, so nothing cross-fades.
- **Do not** set `sharingType = .none`. The default (`.readOnly`) keeps the
  badge visible to Screen Sharing, VNC, and Jump.
- Displays: `NSScreen.screens`; the primary is `screens[0]` (the one with the
  menu bar, by Cocoa convention). Usable area = `visibleFrame`. Scale =
  `backingScaleFactor`. Name = `localizedName`. Re-layout on
  `NSApplicationDidChangeScreenParametersNotification`.
- Coordinates: Cocoa's desktop is in points with a bottom-left origin on the
  primary screen; `backend::macos::Desktop` is the one place that converts
  to and from the core's physical top-left pixels. Inside a screen, one
  point is `backingScaleFactor` pixels, so rects round-trip exactly. Screen
  origins are spread by the largest scale, so with mixed scales the screens
  keep their arrangement and never overlap in physical space.
- Event loop: `nextEventMatchingMask(any, untilDate: deadline,
  NSDefaultRunLoopMode)` waits for the next event or `Shell::timeout()`.
  The IPC thread and the screen-change observer wake it by posting an
  application-defined `NSEvent` from the main dispatch queue. Every event
  the badge doesn't consume goes to `NSApplication.sendEvent`, so a status
  item's menu (M4) works without changing the loop (while a menu is open,
  AppKit tracks it in its own loop, and IPC answers wait until it closes).
- Move mode: `ignoresMouseEvents = false` and `makeKeyWindow` (a
  non-activating panel takes the keyboard without activating the app, so the
  frontmost app stays frontmost). Left-button down on the panel, dragged,
  and up are reported to the core as `DragStart/Move/End` at
  `NSEvent.mouseLocation` in physical pixels; the window server keeps
  sending drags to the app outside the panel. Arrow keys (Shift for 10),
  Return/Enter, and Escape arrive as `keyDown` (`kVK_*` codes); other keys
  are swallowed so nothing beeps. On leaving move mode a key panel is
  ordered out and back in, which returns the keyboard to the active app.
- Session: `SessionGetInfo(callerSecuritySession)` from Security.framework
  (a three-line FFI; no binding crate covers `AuthSession.h`) gives the audit
  session id for the endpoint ([10.4](#104-single-instance-per-graphical-session))
  and whether the session has window server access (`doctor` shows both).
- Menu bar (*M4, to do*): `NSStatusItem` with the same menu, from the same
  `shell::Tray` / `Shell::menu` pair as the Windows tray (11.1.5).
- Autostart: `~/Library/LaunchAgents/io.github.nealhardesty.pawprint.plist`
  with `ProgramArguments = [<path>/pawprint, run]`, `RunAtLoad = true`,
  `LimitLoadToSessionType = Aqua` (GUI sessions only), and
  `KeepAlive = { SuccessfulExit = false }` (restart on crash, not on
  `pawprint quit`). macOS 13+ shows a "Background Items Added" notification
  and lists the agent in System Settings → General → Login Items.
- Gatekeeper: binaries fetched with `curl` don't get the
  `com.apple.quarantine` attribute, so curl-installed binaries run without a
  prompt. Browser downloads are quarantined. The README documents
  `xattr -d com.apple.quarantine`.
- Apple Silicon requires every arm64 executable to carry a signature. The
  Rust toolchain ad-hoc signs at link time. After `lipo` creates the
  universal binary, `make package` re-signs it ad hoc with
  `codesign --force -s -` **(verify)**.

### 11.3 Linux X11

#### 11.3.1 Window

- Created with `x11rb`. EWMH properties:
  `_NET_WM_WINDOW_TYPE = _NET_WM_WINDOW_TYPE_DOCK`,
  `_NET_WM_STATE = ABOVE | STICKY | SKIP_TASKBAR | SKIP_PAGER`,
  `_NET_WM_DESKTOP = 0xFFFFFFFF`, Motif hints with no decorations, and
  `WM_HINTS.input = False`. No `_NET_WM_STRUT`, so no space is reserved.
- If no EWMH window manager is running (`_NET_SUPPORTING_WM_CHECK` absent, as
  in a bare Xvnc session), use an **override-redirect** window and
  `XRaiseWindow` it after map and after display changes.
- Click-through: XFixes `SetWindowShapeRegion(ShapeInput, empty region)`.
  Move mode sets the input region to the badge rectangle.
- Reparenting window managers can frame even a DOCK window. Openbox does,
  and copies the bounding shape to the frame but not the input shape, so
  the frame would catch the clicks (F32). The backend watches
  `ReparentNotify` and sets the same input shape on its frame.
- `WM_NORMAL_HINTS` carries a user-specified position and size, so window
  managers place the window where the core put it. `WM_CLASS` is
  `pawprint`, `Pawprint`; the title is `PAWPRINT`.

#### 11.3.2 Transparency with or without a compositor

- If a compositing manager owns `_NET_WM_CM_S<screen>`, create the window
  with a 32-bit ARGB visual and draw premultiplied pixels.
- If not (common in Xvnc and xrdp sessions with lightweight WMs), use the
  default visual and set the **bounding shape** (`ShapeBounding`) from the
  badge's alpha mask. Because all art is binary-alpha ([4.5](#45-pixel-rules)),
  the result is pixel-identical to the composited version.
- Rebuild the visual path when the compositor appears or disappears
  (`SelectionNotify` / `XFixesSelectSelectionInput` on `_NET_WM_CM_S<n>`):
  the window is destroyed and recreated with the last rectangle and frame.
  The automated tests start and stop `xcompmgr` under a running badge and
  compare both paths pixel by pixel (F17).

#### 11.3.3 Displays and drawing

- RandR 1.5 `GetMonitors`. The primary is the monitor flagged `primary`, else
  the monitor containing (0, 0).
- Usable area = `_NET_WORKAREA` intersected with the target monitor. That
  property is desktop-wide, which is a known EWMH limitation with
  multi-monitor panels.
- Scale: `Xft.dpi` from `RESOURCE_MANAGER` ÷ 96, rounded to 0.25.
- Draw with `PutImage` into the window, split into requests that fit the
  server's maximum request size (byte-swapped for an MSB-first server).
  Redraw on `Expose`. MIT-SHM is not used: the badge is a few kilobytes and
  redraws a few times a minute.
- Re-layout on RandR `ScreenChangeNotify` and `PropertyNotify` for
  `_NET_WORKAREA` and `RESOURCE_MANAGER`.
- Move mode: `GrabPointer` on press, `ConfigureWindow` on motion, ungrab and
  save on release. Arrow keys work through `GrabKeyboard` while in move mode
  (keycodes from `GetKeyboardMapping`, refreshed on `MappingNotify`); if
  another client holds the keyboard, dragging still works.
- The X11 connection is a `calloop` fd source; events that `x11rb` buffers
  while waiting for replies are drained after every loop iteration.
- `x11rb` connects through the filesystem socket or TCP, not the Linux
  abstract socket namespace (F33). Every desktop X server creates the
  filesystem socket.

### 11.4 Linux Wayland (KDE Plasma 6)

- Required globals: `wl_compositor`, `wl_shm`, `zwlr_layer_shell_v1`.
  Optional: `wp_viewporter`, `kde_output_order_v1`.
- Surface: layer `overlay`, namespace `pawprint`, `exclusive_zone = 0`,
  `keyboard_interactivity = none`, and an **empty** input region at all
  times (click-through, F4).
  - `exclusive_zone = 0` makes the compositor place the surface clear of
    panels' exclusive zones, so margins are relative to the usable area (F3).
- **Coordinates.** The core works in physical desktop pixels, but Wayland
  never tells a client where anything is. An output's bounds are its
  xdg-output logical position and size times its scale. Its usable area
  comes from a **work-area probe**: an unmapped layer surface per output,
  anchored to all four edges with `exclusive_zone = 0` and size 0 × 0, which
  the compositor configures to the size left over by panels. Only the size
  is known, so the usable area is placed at the output's origin.
- **Placement.** The badge is anchored to the edges nearest it, using the
  core's own 3 × 3 rule (`layout::snap`), with margins measured from those
  edges (centered axes anchor left or top). The compositor applies margins
  from the real usable area, so the badge lands right even though its
  position inside the output isn't known.
- Primary output: Wayland has no core concept of a primary output. On
  Plasma, `kde_output_order_v1` lists outputs in priority order, and the
  first is the primary. Elsewhere: the configured `display` name, a single
  output, or else the first output by position (not the null output, which
  would leave the core without a usable area to lay out in).
- The output of a layer surface is fixed at creation (F5). The badge is
  recreated when its rectangle moves to another output, when its output
  goes away, and when the compositor closes it.
- Scaling: the scale is the output's physical mode over its xdg-output
  logical size, rounded to 1/120, which equals the compositor's fractional
  scale, so `wp_fractional_scale_v1` adds nothing. With `wp_viewporter`, the
  buffer is the core's physical frame and the viewport destination is its
  logical size. Without it, `wl_surface.set_buffer_scale` with the integer
  output scale (1 when the frame isn't divisible by it).
- Buffers: `wl_shm` `ARGB8888`, which is premultiplied, from a slot pool
  that reuses released buffers. Place, present, and move-mode changes are
  collected and committed once per loop iteration.
- **Move mode** uses a **catcher**: a transparent layer surface on the
  badge's output, anchored to all four edges (so it covers the usable area),
  with `keyboard_interactivity = exclusive`. It takes every click in the
  usable area until move mode ends. Its surface-local coordinates, times
  the scale, from the usable area's origin, are exactly the core's
  coordinates, so a press on the badge rectangle starts a drag, and motion
  and release follow it. The catcher's buffer is one transparent pixel
  stretched by the viewport (a full-size buffer without one).
  - Why not drag the badge surface itself: the implicit grab does deliver
    motion outside the surface, but sway reports positions relative to
    where the surface was **at the press**, so moving the surface under the
    pointer made the drag run away (F7). The catcher never moves.
  - Why exclusive: with `on_demand`, arrow keys would need a click on the
    badge first, and a click ends move mode. Exclusive keyboard focus is the
    Wayland equivalent of the X11 keyboard grab, held at most 30 seconds.
- Keys: the backend binds `wl_keyboard` without libxkbcommon and reads the
  compositor's keymap just far enough to find the keycodes of Left, Right,
  Up, Down, Return, KP_Enter, and Escape, so virtual keyboards with their
  own keymaps (remote desktop servers, `wtype`) work. Without a usable
  keymap it falls back to the evdev codes. Shift is XKB modifier 0.
- Tray: StatusNotifierItem via `ksni` (`backend::sni`), shared with X11 and
  GNOME. Plasma supports it natively. The item (`Id` `pawprint`, title
  `PAWPRINT`, the tooltip from `TrayState`) shows the core's menu model
  over `com.canonical.dbusmenu`: submenus for children, checkmark items for
  checks. A left click opens the menu (`ItemIsMenu`), as on Windows and
  macOS. The icon is the rest frame in its crate at 32 × 32 and 64 × 64
  (nearest neighbor), ARGB32 in network byte order; the host scales it.
  Menu callbacks run on ksni's thread and post the command into the
  `calloop` channel that IPC uses (`linux::Inbox::Menu`), so both reach
  `Shell::menu`/`Shell::request` on the loop. The tray joins after the
  first frame is on screen, so the session bus never delays the badge. It
  is spawned with `assume_sni_available`: without a StatusNotifierWatcher
  it waits quietly for one (a panel that starts later or restarts); without
  a session bus there is no tray, and `doctor` reports either case as a
  problem. `tray = false` stops the item and a reload brings it back.

### 11.5 GNOME Wayland (Ubuntu)

**Targets:** Ubuntu 24.04 LTS (GNOME 46) and newer, including 26.04 LTS,
and upstream GNOME 46+.

Mutter does not implement layer-shell (F2). PAWPRINT supports GNOME by
**reusing the X11 backend through XWayland**, with a small `gnome` profile.
This path needs no extension and nothing extra to install. GNOME sessions
set `DISPLAY` and start Xwayland on demand, and still do now that GNOME has
dropped its X11 session (F31).

#### 11.5.1 GNOME profile of the X11 backend

Selected automatically when the session is Wayland, layer-shell is missing,
and `XDG_CURRENT_DESKTOP` contains `GNOME` ([3.2](#32-linux-backend-selection-backend--auto)).
Differences from [11.3](#113-linux-x11):

| Concern | Behavior on GNOME |
|---|---|
| Stacking | `_NET_WM_WINDOW_TYPE_DOCK` + `_NET_WM_STATE_ABOVE`. Mutter keeps one stack for X11 and Wayland windows, so an above-state XWayland window stays above normal Wayland windows (F25). |
| Click-through | Empty XFixes input shape, as on X11. Mutter honors X11 input shapes (F26). |
| Transparency | Mutter always composites, so the ARGB visual is always used and the shape fallback never runs. |
| Primary display | RandR primary as reported by Xwayland, which mirrors Mutter's primary monitor (F27). |
| Usable area | `_NET_WORKAREA` as published by Mutter (excludes the top bar and the Ubuntu Dock). |
| Scaling | Crisp at 100% and integer scales. With **fractional scaling** (offered in Ubuntu's Settings → Displays), Mutter renders XWayland windows at 1× and upscales them, which blurs pixel art. GNOME 47+ fixes this with Mutter's `xwayland-native-scaling` experimental feature (F28). `pawprint doctor` detects the situation and prints the exact `gsettings` command. |
| Tray | Ubuntu ships the AppIndicator/KStatusNotifierItem extension enabled, so the shared `ksni` tray works. Upstream GNOME has no tray, so the CLI is the interface there and `doctor` says so (F29). |
| Autostart | XDG autostart, same as every Linux desktop |

In code, the profile is a flag on the X11 backend that forces the ARGB
visual; everything else in the table is the plain X11 path, which already
behaves that way under Mutter. `status` reports the backend as
`x11 (gnome profile)`. The fractional-scaling check in `doctor` reads the
monitor scales from Wayland (physical mode over xdg-output logical size),
reads `org.gnome.mutter experimental-features` with `gsettings`, and, when
a monitor is fractional and `xwayland-native-scaling` is off, prints the
`gsettings set` command with the current features plus that one, or says
that GNOME 46 has no fix (use 100% or 200%).

#### 11.5.2 Remote sessions on GNOME

GNOME Remote Desktop serves RDP and VNC. In **desktop sharing** mode it
mirrors the console session. In **remote login** mode (Ubuntu 24.04+) it
creates headless GNOME sessions, and each gets its own PAWPRINT through XDG
autostart **(verify)**. Both modes stream Mutter's screencast, which includes
XWayland windows.

#### 11.5.3 Plan B: the presenter extension

Plan B is built **only if** the M1 GNOME spike fails the backend contract or
the fractional-scaling check. It is designed now so a failure doesn't send
the design back to the drawing board.

- A thin GNOME Shell extension, `pawprint@nealhardesty.github.io` (about 200
  lines of GJS), that contains **no logic**. It only shows pixels it's given.
- It owns `io.github.nealhardesty.Pawprint.Presenter` on the session bus:
  - methods `Present(width, height, scale, pixels)` (the same premultiplied
    BGRA buffer every backend receives, [10.6](#106-code-sharing-dry)),
    `Place(x, y)` (logical), and `SetInteractive(bool)`;
  - signals `Pointer(kind, x, y)`, `Key(keysym)`, and
    `DisplaysChanged(json)`.
- It draws one `St.Widget` with `Clutter.Image` content, using a nearest
  scaling filter so pixels stay crisp at fractional scales. The widget is
  added with `Main.layoutManager.addTopChrome` and is non-reactive
  (click-through) outside move mode.
- On the Rust side it's one more `Surface` implementation (`gnome` backend,
  over `zbus`, which the tray already depends on). Layout, rendering, and
  move mode stay in `pawprint-core`.
- Installed by `pawprint gnome-extension install`, which writes the embedded
  extension files into `~/.local/share/gnome-shell/extensions/` and enables
  it. On Wayland, a newly installed extension loads only after logging out
  and back in.
- Selection: if the presenter's bus name is present, use it. Otherwise use
  XWayland.
- Maintenance cost: `metadata.json`'s `shell-version` list needs a bump for
  each GNOME release.

---

## 12. Mascot assets

### 12.1 Art is code

Every bundled mascot is a **text source file**: palette-keyed pixel grids
plus animation data in TOML. The `pawprint-sprite` crate compiles sources to
sprites at build time. There are no committed binary sprites.

Why text:

- **Diffable and reviewable.** A changed whisker is a one-character diff in a
  pull request.
- **Authorable by anyone.** Humans in any editor, coding agents directly, and
  pixel-art tools through PNG import/export ([12.7](#127-png-bridges)).
- **DRY.** Shared **bases**, **recolors**, **motion templates**, and **props**
  let 100 distinct mascots reuse the same blinks, bobs, and body plans without
  copying pixels.
- **One compiler everywhere.** `build.rs`, user mascots at runtime, `pawdev`,
  and `pawprint mascot check` all use the same crate, so they can't disagree.

### 12.2 Building blocks

| Block | Location | What it is |
|---|---|---|
| **Master palette** | `assets/palette.toml` | 32 named colors ([12.9](#129-master-palette)). All art uses these names, never raw hex. `make palette` writes `assets/palette.gpl` from it for GIMP, Aseprite, and LibreSprite; a test fails if the `.gpl` is stale. |
| **Crate** | `assets/crate/crate.toml` | Crate back, crate front, label strip, and nameplate parts, drawn with accent-ramp glyphs so any accent tints them |
| **Bases** | `assets/bases/<name>.toml` | Shared body plans in *role* colors (fur, fur-shadow, belly, ...) with standard layers. A mascot `extends` a base, maps roles to palette colors, and adds or overrides layers. |
| **Motion templates** | `assets/motions/<name>.toml` | Reusable actions written as layer operations (blink, bob, duck, ...). They work on any mascot that has the layers they need. |
| **Style guide** | `docs/ART.md` | How to author a mascot on a base, with worked examples and the art bible's practical gotchas |
| **Props** | `assets/props/<name>.toml` | Small shared sprites: `zzz`, `heart`, `bang`, `question`, `note`, `steam`, `sparkle`, `seed`, `leaf`, `bubble`, `star`, `sweat`, `puff`, `pebble`, `bamboo` |
| **Mascots** | `assets/mascots/<id>/mascot.toml` | One file per mascot: identity, palette mapping, layers, and actions |

### 12.3 Mascot source format

```toml
id = "owl"
name = "Owl"
category = "birds"                  # mammals | cats-dogs | birds | aquatic | critters | oddballs
aliases = ["owls", "hoot"]          # the id is always an implicit alias
accent = "violet"
fallback = true                     # eligible for the deterministic fallback (default true)
nocturnal = true                    # prefers `when = "night"` actions after dark (default false)
extends = "bird-perched"            # optional base

# Glyph → master palette color. Base role glyphs are remapped here.
[palette]
"." = "clear"
"k" = "ink"
"f" = "brown-2"        # role: fur/feathers
"F" = "brown-3"        # role: shadow
"l" = "brown-1"        # role: light
"w" = "cream"          # role: facial disc
"e" = "amber"          # role: iris
"h" = "white"          # role: eye highlight
"b" = "orange-2"       # role: beak

# Layers. A name with a dot is a variant of a layer slot: `eyes` is the
# default and `eyes.closed` replaces it when an action asks.
# `at` places a smaller grid; omitted means a full 32×32 grid at (0, 0).
[layers."eyes"]
at = [9, 7]
follows = "head"       # takes the head's offsets: `head+1,0` moves the eyes too
grid = """
.kkk..kkk.
kehk..kehk
.kkk..kkk.
"""

[layers."eyes.closed"]
at = [9, 7]
grid = """
..........
.kkk..kkk.
..........
"""

[layers."tufts"]       # a new layer slot this mascot adds to the base
at = [6, 0]
grid = """
kk..............kk
kfk............kfk
"""

# Rest pose: every default layer, in base order plus additions. Override with
# `rest = ["body", "head", ...]` if needed.

[[actions]]
use = "blink"          # a motion template
weight = 6

[[actions]]
name = "head-turn"     # signature action, written as layer ops
weight = 3
frames = [
  { ops = ["eyes=closed"], ms = 160 },
  { ops = [], ms = 200 },
  { ops = ["head+1,0"], ms = 220 },
  { ops = ["head+2,0"], ms = 900 },
  { ops = ["head+1,0"], ms = 200 },
]

[[actions]]
name = "full-spin"     # rare action, using custom full-frame grids
weight = 1
when = "night"
frames = [
  { grid = "spin-1", ms = 150 },
  { grid = "spin-2", ms = 150 },
  { grid = "spin-3", ms = 150 },
  { ops = [], ms = 400 },
]

[frames."spin-1"]
grid = """
...32 rows of 32 glyphs...
"""
```

**Layer ops** (used by mascot actions and templates):

| Op | Meaning |
|---|---|
| `slot=variant` | Swap a slot to a variant: `eyes=closed` |
| `slot+dx,dy` | Offset a slot: `head+1,0` |
| `all+dx,dy` | Offset the whole mascot (the crate never moves) |
| `-slot` | Hide a slot |
| `+slot` | Show an optional slot that's hidden at rest (`mouth.open`) |
| `prop:name@x,y` | Draw a shared prop at mascot coordinates |
| `flipx` | Mirror the mascot horizontally |

Details the compiler enforces:

- Layer names are kebab-case `slot` or `slot.variant`; `all` is reserved. A
  layer without `at` must be a full 32 × 32 grid, and a layer with `at` must
  fit inside 32 × 32. Layer order is drawing order: base layers first, then
  the mascot's additions; a mascot layer with a base layer's name replaces it
  in place.
- A slot with only variants (`mouth.open` and no `mouth`) is hidden at rest.
  `+slot` shows a slot's default layer; `+slot.variant` and `slot=variant`
  show that variant.
- Offsets are signed integers after a `+`: `head+-1,0` moves the head left.
- **`follows = "<slot>"`** on a layer makes its slot take that slot's
  offsets as well as its own, so `head+1,0` carries the eyes, ears, nose, and
  mouth that follow `head`. Chains are allowed (`eyes` follows `disc`
  follows `head`); loops and unknown slots are errors. Every layer of a slot
  that sets `follows` must name the same slot. When a mascot replaces a base
  layer and doesn't set `follows`, it keeps the base layer's. `follows`
  affects offsets only, not visibility.
- Props are drawn after the mascot (and after `flipx`) at mascot
  coordinates; `all+` does not move them.
- `ms` defaults to 200 and `weight` to 5. A template action takes the
  template's `weight` and `when` unless the mascot's `[[actions]]` entry sets
  its own, and `name` defaults to the template's name.
- Grids: one glyph per pixel, any non-whitespace character; leading
  whitespace on each row is ignored, so grids may be indented. A base's
  `[palette]` supplies defaults that the mascot's `[palette]` overrides.

**Custom frames.** When layer ops can't express a pose (a curl, a spin, a
roll), an action frame can reference a full 32 × 32 grid from the mascot's
`[frames.<name>]` table with `{ grid = "<name>", ms = ... }`. Each mascot has a few at most. A grid frame may also carry `all+`, `flipx`, and `prop:` ops, but no slot ops.

**Bases.** A base file has the same format without identity fields. Its
`[palette]` values are master palette names: a neutral default so the base
alone renders as a recognizable generic animal. Its glyphs are *roles*. The mascot's `[palette]` maps them, so one cat base
yields a black Maine Coon, a buff tabby, and a gray cat. Mascots then add
pattern layers (tabby stripes, cow-cat patches, tuxedo bib) and species
features. **A base is a starting point, not an identity:** the readability
review ([12.6](#126-production-process)) fails any mascot that reads as a
recolor of its neighbor. Every base also compiles **on its own** as a stand-in
mascot (`pawprint_sprite::compile_base`) with every template whose `needs` it
meets; the build fails if any base doesn't, so a broken base is caught before
any mascot extends it. Shared role glyphs and standard slots are in
[12.8](#128-bases).

### 12.4 Motion templates

```toml
# assets/motions/duck.toml
name = "duck"
summary = "Sinks into the crate, waits, pops back up"
needs = []                 # slots/variants the mascot must define
frames = [
  { ops = ["all+0,2"], ms = 120 },
  { ops = ["all+0,6"], ms = 120 },
  { ops = ["all+0,10"], ms = 900 },
  { ops = ["all+0,4"], ms = 120 },
  { ops = [], ms = 200 },
]
```

The launch template set:

| Template | Needs | Motion |
|---|---|---|
| `blink` | `eyes.closed` | Eyes close for 140 ms |
| `double-blink` | `eyes.closed` | Two quick blinks |
| `bob` | — | Whole body dips 1 px and returns |
| `duck` | — | Sinks behind the crate front, pops back up |
| `peek` | — | Ducks, then rises with only the eyes showing for a beat |
| `hop` | — | 2 px jump with a 1 px squash on landing |
| `look-left` / `look-right` | `eyes.left` / `eyes.right` | Glance and hold |
| `ear-flick` | `ears.flick` | One ear twitches |
| `tail-flick` | `tail.flick` | Tail flicks over the crate edge |
| `sniff` | `nose.sniff` | Nose twitches twice |
| `shiver` | — | ±1 px horizontal jitter, 4 frames |
| `yawn` | `mouth.open`, `eyes.closed` | Mouth opens slowly, eyes squeeze |
| `doze` | `eyes.closed` | Eyes close, `zzz` prop rises; night only |
| `startle` | — | `bang` prop, 1 px hop |
| `curious` | `head` | `question` prop, head tilt via `head+1,0` |
| `content` | `eyes.happy` | Happy eyes, `heart` prop |

That is 17 template files (`look-left` and `look-right` are separate). `doze`
sets `when = "night"`. `peek` holds at `all+0,9`, so mascot rows 0–10 show
over the rim: eyes belong in rows 8–10. Props in templates land in the top
right (`zzz` from x 21, `bang` at x 26, `question` at x 24, `heart` at x 22),
so keep that corner for them where the silhouette allows.

### 12.5 Animation extras that cost nothing at runtime

- **Signature action:** every mascot has at least one action unique to it
  ([Appendix A](#appendix-a-launch-catalog)).
- **Rare actions:** an action with `weight = 1` among weights of 5–10 plays
  about once an hour at default timing. Every showcase mascot has one. For
  example, the owl spins its head all the way round, the armadillo rolls off
  the crate and back, the ghost drifts *through* the crate front, and the CRT
  briefly shows `> hello_`.
- **Day and night:** `when = "day" | "night" | "any"` (default `any`). Night
  is 20:00–06:00 by the local clock. Nocturnal mascots (owl, raccoon, moth,
  opossum, cat-black) are livelier at night: their `when = "night"` actions
  weigh 3× at night (`anim::NOCTURNAL_NIGHT_BOOST`). Everyone else gets
  `doze` at night. No network or location is involved.

### 12.6 Production process

How 100 complete mascots get made:

1. **Pipeline first.** The master palette, crate, props, the 16 motion
   templates, and the bases ([12.8](#128-bases)) land before any catalog
   mascot.
2. **Showcase 12, bespoke.** Each gets at least two template actions, a
   signature action, and a rare action, and goes through the review below
   twice.
3. **Waves by base.** The remaining 88 mascots are made in waves grouped by
   base (all cats, all dogs, the ungulates, and so on). Grouping makes
   siblings directly comparable, so "too similar" is caught inside a wave.
   Every catalog mascot needs at least one template action and one
   signature action.
4. **Authoring.** Coding agents and humans write and edit `mascot.toml`
   directly, following the art bible ([12.10](#1210-art-bible)). Artists can
   draw in any pixel tool and import ([12.7](#127-png-bridges)).
5. **Review tools.**
   - `make preview MASCOT=<id>` animates one mascot in a window, cycling
     every action.
   - `make contact-sheet` renders every mascot's rest frame at 1× and 2×,
     **with labels hidden**, grouped by base, into `docs/gallery.png`.
   - `make contact-sheet ANIMATED=1` also writes `docs/gallery.gif` with
     every action playing: each mascot loops rest, then each action in turn
     with 700 ms of rest between.
   - `make building-blocks` renders `docs/building-blocks.png`: the crate in
     all 16 accents with a label, every prop at 4×, and a grid of every base
     with every motion template (each cell shows the template's most-changed
     frame; `-` marks a template whose `needs` the base lacks).
   - The checklist below is the art section of `.github/pull_request_template.md`.
6. **Review checklist** (every mascot, recorded in the PR):
   - [ ] **Squint test:** recognizable at 1× (32 px) with the label covered
   - [ ] Distinct from every sibling on the same base
   - [ ] Species-defining feature exaggerated (see the art bible)
   - [ ] Face and silhouette in rows 0–19
   - [ ] Each action reads in under a second and returns cleanly to rest
   - [ ] Signature action matches its Appendix A brief
   - [ ] `make check-assets` is clean, with no warnings
7. **Provenance.** All art is original to this repository and MIT licensed.
   Nothing is traced from or derived from existing games, characters, or
   other copyrighted art, including the desktop pets that inspired the idea.

### 12.7 PNG bridges

- `make art-export MASCOT=<id>` writes `out/art/<id>/`: one PNG per layer
  and variant at the layer's own size, named `<slot>.png` or
  `<slot>.<variant>.png` (base layers included, after `extends` is merged;
  each line printed gives the layer's `at`), plus `frames.png`, a strip of
  every distinct compiled 32 × 32 frame with rest first, for editing in
  Aseprite, LibreSprite, or Piskel.
- `make art-import MASCOT=<id> PNG=<file>` converts a PNG layer back into a
  grid and **prints** a ready-to-paste `[layers."<name>"]` table (it never
  edits `mascot.toml`). The layer name is the file name without `.png`; a
  layer the mascot already has keeps its `at`. Transparent pixels take the
  glyph mapped to `clear`. Each opaque color snaps to the nearest master
  palette entry (squared RGB distance), then to the mascot glyph mapped to
  that name; inexact colors and colors with no glyph (written as `?`) are
  reported on stderr. Partial alpha is an error.
- **User mascots may skip the text format entirely:** a directory holding a
  `mascot.toml` with `sprite = "sprite.png"` (a horizontal strip of 32 × 32
  frames) and actions whose `frames` are frame indices is accepted as-is.
  `rest` is a frame index (default 0) and `duration_ms` (default 200) sets
  every frame of an action. PNG mascots take no `extends`, `palette`,
  `layers`, or motion templates, and any pixel with partial alpha is an
  error. `pawprint mascot export <id> <dir>` produces this simpler form.

### 12.8 Bases

| Base | Body plan | Members (guide, not a rule) |
|---|---|---|
| `cat-sit` | Sitting cat, ears up, tail slot | All 12 cats |
| `dog-sit` | Sitting dog; floppy `ears` by default, with `ears.prick` and `ears.fold` to list in `rest` instead; snout | All 12 dogs |
| `ungulate-head` | Head and shoulders over the crate; antler/horn slot | moose, elk, deer, bison, goat, llama |
| `bear` | Round ears, broad muzzle, paws on rim | bear, panda, polar-bear, koala |
| `rodent-upright` | Upright small mammal, paws to chest | marmot, chipmunk, squirrel, mouse, hamster, beaver |
| `small-quadruped` | Pointed face, low body | fox, raccoon, red-panda, badger, skunk, opossum, hedgehog, otter |
| `big-cat` | Large feline head, mane slot | lion, tiger |
| `primate` | Rounded head, long arms on rim | monkey, sloth |
| `bird-perched` | Upright bird, big eyes, beak slot | owl, crow, parrot, eagle, pigeon, toucan, kiwi, hummingbird |
| `bird-water` | Long neck or waddler body | duck, swan, puffin, penguin, pelican, flamingo, heron, chicken |
| `pinniped` | Smooth head, whiskers, flippers on rim | walrus, seal |
| `sea-creature` | Body rising from water inside the crate (water line layer) | whale, dolphin, shark, narwhal, pufferfish, axolotl |
| `critter` | Small body, big eyes | gecko, frog, snake, tortoise, snail |
| `bug` | Segmented body, wing and antenna slots | moth, beetle, bee |
| *(bespoke)* | No base | armadillo, rabbit, capybara, elephant, wolf, octopus, jellyfish, crab, and all 8 oddballs |

Conventions every base follows, so mascots and templates work across them
(each base file's header lists its own roles and slots):

- **Shared role glyphs:** `.` clear, `k` ink; `l` `f` `F` the main
  material's light, base, and shadow; `w` `m` `M` the secondary material
  (belly, muzzle, chest, facial disc); `e` iris; `h` eye highlight; `n`
  nose; `p` inner ear, tongue, and mouth; `b` `B` beak or bill. Bases add
  documented extras (antlers, water, markings).
- **Standard slots**, where the animal has them: `tail` (+`flick`), `body`,
  `head`, `ears` (+`flick`), `eyes` (+`closed`, `left`, `right`, `happy`),
  `nose` (+`sniff`), `mouth.open` (hidden at rest; for `yawn`), and `paws`
  (or feet, flippers, fins) on the rim. Face slots follow `head`.
- **Geometry:** eyes in rows 8–10, face in rows 4–16, paws on the rim in
  rows 17–19, the body continuing to row 31 behind the crate front so ducks
  and hops never show a gap.
- `make building-blocks` shows every base and every template applied to it.

### 12.9 Master palette

Thirty-two named colors, **frozen as of M2** (renames break sources).
`clear` is transparency and isn't counted. `assets/palette.gpl` is generated
from `palette.toml` (`make palette`).

| Name | Hex | Name | Hex | Name | Hex | Name | Hex |
|---|---|---|---|---|---|---|---|
| `ink` | `#1b1b1f` | `brown-1` | `#d9b48a` | `red-1` | `#e86a5f` | `blue-1` | `#bfe3f2` |
| `white` | `#f7f5ef` | `brown-2` | `#a8784d` | `red-2` | `#b23a3a` | `blue-2` | `#5b9bd5` |
| `cream` | `#f2efe6` | `brown-3` | `#74502f` | `pink-1` | `#f4b7c4` | `blue-3` | `#2f5f8f` |
| `grey-1` | `#d4d2cc` | `brown-4` | `#4a311d` | `pink-2` | `#d77a91` | `purple-1` | `#b39ddb` |
| `grey-2` | `#9a9893` | `orange-1` | `#f6c27a` | `green-1` | `#a8cf6a` | `purple-2` | `#6e4b9e` |
| `grey-3` | `#5d5c59` | `orange-2` | `#e8912d` | `green-2` | `#5f9a3f` | `teal` | `#2a8c87` |
| `grey-4` | `#34343a` | `orange-3` | `#b8612a` | `green-3` | `#2f6b4f` | `amber` | `#f0a830` |
| `fur-black` | `#2b2833` | `yellow-1` | `#f9e27d` | `yellow-2` | `#e2b43c` | `blush` | `#ee9a9a` |

`ink` is reserved for outlines and pupils. `fur-black` exists so a black cat
reads as fur, not as a hole in the outline.

### 12.10 Art bible

- **Grid:** 32 × 32. The mascot spans roughly columns 2–29. The face sits in
  rows 4–16, and rows 18–20 are where paws meet the crate rim.
- **Light** from the top-left. Each material uses a 3-step ramp (light, base,
  shadow). No dithering at 1×.
- **Outline:** 1 px `ink` around the silhouette. Interior lines use the
  material's shadow color, never `ink`.
- **Eyes:** 2 × 2 with a 1 px `white` highlight for big-eyed mascots, 1 × 2
  for small ones. Eyes are the main source of expression, so give every
  mascot `eyes.closed` and, where it fits, `eyes.left`, `eyes.right`, and
  `eyes.happy`.
- **Interact with the crate.** Paws, flippers, beaks, and tails over the rim
  make a mascot feel like it lives there.
- **Exaggerate the defining feature:** moose paddle antlers and dewlap; elk
  tall branching tines; walrus tusks and whiskers; chipmunk back stripes;
  armadillo banded shell; marmot round body and small ears; owl facial disc
  and ear tufts; Maine Coon ear lynx tips and ruff; cow cat asymmetric patches
  on white; tuxedo bib and white paws.
- **Motion:** at most 2 px displacement in template actions. Signature
  actions may go further. Use 3–8 frames, with holds longer than moves, and a
  settle frame before returning to rest. No full-body flashes or color
  cycling. Calm beats busy.
- **Colors:** at most 16 per mascot, crate excluded.
- Worked examples and the practical gotchas are in [`docs/ART.md`](docs/ART.md).

### 12.11 Validation rules

Rules 1–8 are enforced by `pawprint-core`'s `build.rs` (so any build,
including `make test`, fails on a bad asset), by `make check-assets`, and by
`make package` for bundled mascots, and by `pawprint mascot check` for user
mascots. Rule 9 is enforced **only** by `make package`
(`pawdev check-assets --package`): `make test` and `make check-assets`
report the gates without failing, because the catalog stays small until
M3. The build also fails if no mascot is eligible for the fallback.

1. The source parses. `id` matches the directory and the ID grammar.
   `extends`, `use`, `follows`, and `prop:` references resolve. Every base
   compiles on its own ([12.3](#123-mascot-source-format)).
2. Every grid row has the declared width, and every grid glyph is in the
   palette. Every palette value is a master palette name (bundled) or a
   name/`#rrggbb` (user).
3. Every op references an existing slot or variant. Every template's `needs`
   are satisfied.
4. Compiled frames: every alpha is 0 or 255 (always true for grids, checked
   for PNG mascots). More than 16 colors is a warning, and a warning in a
   bundled mascot fails the build.
5. At least one action. Durations 80–2000 ms. Each action ≤ 4000 ms total.
   Every bundled mascot has a signature (non-template) action.
6. The rest frame has opaque pixels in rows 0–19.
7. `accent` resolves.
8. Squashed aliases are unique across the catalog (bundled check). For user
   mascots, collisions are a warning, and the user mascot wins.
9. **Catalog gates (bundled only, `make package` only):** ≥ `minimum`
   (100) valid entries, every `required` id present, and every showcase
   (required) mascot has ≥ 2 template actions, a signature action (not from
   a template, weight above 1), and a rare action (weight 1).

### 12.12 Catalog file

`assets/catalog.toml` holds catalog-wide data:

```toml
version = 1
required = [
  "owl", "moose", "marmot", "elk", "armadillo", "bear", "walrus", "chipmunk",
  "cat-maine-coon-black", "cat-buff-tabby", "cat-cow", "cat-tuxedo",
]
minimum = 100
```

### 12.13 Embedding

`pawprint-core`'s `build.rs` uses `pawprint-sprite` (as a build-dependency)
to compile every bundled source into indexed frame data (32 × 32 palette
indices per distinct frame, identical frames shared) and validate the
catalog. The result, a `pawprint_sprite::Bundle` (accents, font, crate,
mascots, catalog gates), is serialized with `postcard` to
`$OUT_DIR/bundle.bin`, embedded with `include_bytes!`, and decoded once on
first use (`pawprint_core::catalog::bundle()`). The binary needs no asset
files at runtime. Frames stay indexed; a frame is expanded to RGBA only
when it is drawn. The bundle also embeds the library as source text (palette, accents,
bases, motion templates, props: `Bundle::library`), so text-format user
mascots compiled at load time by the same crate can extend bases and use
templates and props (`pawprint_core::catalog::library()`).

---

## 13. Repository layout

```
pawprint/
├── Makefile                  # the only entry point for humans and CI
├── Cargo.toml                # workspace; [workspace.package] version is the release version
├── .cargo/config.toml        # rust-lld as the musl linker (14.4)
├── rust-toolchain.toml       # pinned stable toolchain + targets
├── crates/
│   ├── pawprint-core/        # pure logic + build.rs asset embedding; no OS deps
│   ├── pawprint-sprite/      # sprite source compiler (build.rs, runtime, pawdev)
│   └── pawprint/             # lib + bins pawprint, pawprintw (Windows): cli, ipc, instance, autostart, backend/{windows,macos,linux,x11,wayland}
├── tools/
│   └── pawdev/               # dev-only binary: check-assets, contact-sheet, art import/export, golden, package, publish
├── assets/
│   ├── catalog.toml
│   ├── palette.toml          # master palette (12.9); pawdev also emits palette.gpl for pixel tools
│   ├── accents.toml          # accent palette (Appendix B)
│   ├── crate/crate.toml      # crate back/front, label strip, nameplate (text source)
│   ├── font/font.toml        # 5x7 bitmap font (text source)
│   ├── bases/                # shared body plans (12.8)
│   ├── motions/              # motion templates (12.4)
│   ├── props/                # shared props (12.2)
│   └── mascots/<id>/mascot.toml
├── installers/
│   ├── install.sh
│   └── install.ps1
├── tests/
│   └── golden/               # expected badge renders (PNG)
├── extras/
│   └── gnome-presenter/      # Plan B GNOME Shell extension (only if needed, 11.5.3)
├── docs/
│   ├── ART.md                # mascot style guide with worked examples
│   ├── gallery.png           # generated contact sheet (make contact-sheet; gallery.gif with ANIMATED=1)
│   └── building-blocks.png   # every base x every template, props, accents (make building-blocks)
├── spikes/
│   └── f22-static-musl/      # standalone spike crate (not a workspace member); make spike-f22
├── README.md  SPEC.md  ROADMAP.md  AGENTS.md  LICENSE
└── .github/workflows/ci.yml  # calls make targets only
```

Backends compile only on their platform (`#[cfg(target_os = ...)]`). Linux
builds include `x11` (with its GNOME profile), `wayland`, and the shared
`linux` module.

---

## 14. Build system

### 14.1 Rules

- The root `Makefile` is the interface. CI calls make targets, never raw
  `cargo`.
- Every target has a `## description` comment. `make` with no arguments runs
  `make help`, which prints them.
- **Recipes are single commands** that invoke `cargo` or `pawdev`. Real logic
  lives in Rust (`tools/pawdev`), not in shell, so recipes behave the same
  under `sh` and `cmd.exe`.
- **GNU Make 3.81 compatible.** macOS ships GNU Make 3.81. No `.ONESHELL`,
  `.RECIPEPREFIX`, `!=`, `$(file ...)`, or other 3.82+/4.x features.
- OS detection: `ifeq ($(OS),Windows_NT)` (the `OS` variable is set by
  Windows itself), otherwise `uname -s`.
- On Windows, GNU Make runs recipes with `sh.exe` if one is on `PATH` (for
  example from Git for Windows), and with `cmd.exe` otherwise. Single-command
  recipes make this irrelevant.

### 14.2 Targets

| Target | Description |
|---|---|
| `help` (default) | List targets |
| `build` | Debug build of `pawprint` (and `pawprintw` on Windows) |
| `run` | Build and run the badge in the foreground; `ARGS="..."` passes arguments |
| `preview` | Open a preview window: `make preview MASCOT=owl SIZE=128` (`pawprint preview`) |
| `power-check` | Linux: measure the release badge under Xvfb against [7.2](#72-power-budget); `POWER_SECONDS=120` per mode |
| `test` | Unit tests, golden render tests, and asset validation |
| `check-assets` | Validate the mascot catalog only |
| `lint` | `cargo fmt --check` and `clippy -D warnings` |
| `fmt` | Format the code |
| `golden` | Regenerate golden images after an intentional rendering change |
| `contact-sheet` | Render every mascot, labels hidden, grouped by base, into `docs/gallery.png` (`ANIMATED=1` adds `docs/gallery.gif`) |
| `building-blocks` | Render the crate accents, props, and every base with every motion template into `docs/building-blocks.png` |
| `palette` | Regenerate `assets/palette.gpl` from `assets/palette.toml` |
| `art-export` | Export a mascot's layers and frames as PNGs: `make art-export MASCOT=owl` |
| `art-import` | Convert a PNG layer into a grid: `make art-import MASCOT=owl PNG=eyes.png` |
| `ci` | `lint` + `test` (what CI runs on every OS) |
| `release` | Optimized build for this platform (universal on macOS, musl on Linux) |
| `package` | `release` + archive + `.sha256` into `dist/`; enforces catalog gates |
| `publish` | Upload `dist/*` and installers to the GitHub release `v$(VERSION)`, creating it if needed (needs `gh`) |
| `install` | Install the local release build for this user and enable autostart |
| `uninstall` | Remove the local install and autostart (config is kept) |
| `doctor` | Run `pawprint doctor` from the local build |
| `clean` | Remove `target/` and `dist/` |
| `check-all-targets` | `cargo check` of the whole workspace for every release target, from any OS (needs no linkers) |
| `spike-f22` | Rebuild the F22 spike ([14.4](#144-static-linux-binaries)) for x86_64 and aarch64 musl |
| `test-x11` | Linux: the X11 backend under Xvfb, bare, with `xcompmgr`, and with `openbox` ([16.1.1](#1611-display-server-tests-linux)) |
| `test-wayland` | Linux: the Wayland backend under headless sway ([16.1.1](#1611-display-server-tests-linux)) |
| `test-linux-display` | Both of the above (what the Linux display CI job runs) |
| `test-windows-display` | Windows: the real badge on this desktop ([16.1.3](#1613-desktop-test-windows)); a `windows-latest` CI job runs it |

Targets whose features don't exist yet stop with
`make <target>: not implemented yet (Mx)` from `pawdev todo`, naming the
milestone in [`ROADMAP.md`](ROADMAP.md). `make package` already runs the
catalog gates first.

### 14.3 Toolchains per OS

| OS | Requirements |
|---|---|
| Linux | `rustup`; `make`; targets `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`. Both link with rustup's own `rust-lld` (decided in M0, F22): no cross linker, `musl-gcc`, or `cargo-zigbuild` |
| macOS | `rustup`; Xcode Command Line Tools (provides `make` 3.81, `lipo`, `codesign`); targets `aarch64-apple-darwin`, `x86_64-apple-darwin` |
| Windows | `rustup` with the MSVC toolchain; Visual Studio Build Tools (C++ workload); GNU Make (`winget install ezwinports.make`); target `x86_64-pc-windows-msvc` |

`rust-toolchain.toml` pins the channel and lists the targets, so `rustup`
installs them on first build.

### 14.4 Static Linux binaries

`x11rb`, `wayland-client` (with the pure-Rust backend, no `libwayland`
linking), `smithay-client-toolkit`, `calloop`, `zbus`, and `ksni` are pure
Rust. The Linux release is therefore a static `musl` binary with **no
shared-library dependencies**, so it runs on any distribution, with no glibc
baseline to manage (confirmed in M0, F22).

How: rustc ships the musl CRT and libc for both musl targets, and rustup
ships `rust-lld`. `.cargo/config.toml` sets `linker = "rust-lld"` for
`x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`. No RUSTFLAGS,
`musl-gcc`, zig, or sudo are needed. The default `cc` linker happens to work
for x86_64 when a host gcc exists, but fails for aarch64
(`unrecognized option '--fix-cortex-a53-843419'`), so rust-lld is used for
both. Keep the C-pulling crate features off (10.3). `make spike-f22`
rebuilds the proof: a binary that really calls x11rb, wayland-client with
sctk, calloop, zbus, and ksni.

---

## 15. Packaging, release, and install

### 15.1 Versioning

SemVer. The version lives in `[workspace.package].version` in the root
`Cargo.toml`. Tags are `v<version>`. `make publish` reads the version from
there, so there is no `VERSION=` argument to get wrong.

### 15.2 Release assets

| Asset | Contents |
|---|---|
| `pawprint-<ver>-x86_64-pc-windows-msvc.zip` | `pawprint.exe`, `pawprintw.exe`, `README.md`, `LICENSE` |
| `pawprint-<ver>-universal-apple-darwin.tar.gz` | `pawprint`, `README.md`, `LICENSE` |
| `pawprint-<ver>-x86_64-unknown-linux-musl.tar.gz` | `pawprint`, `README.md`, `LICENSE` |
| `pawprint-<ver>-aarch64-unknown-linux-musl.tar.gz` | same |
| `<archive>.sha256` | One sidecar per archive (`sha256sum` format) |
| `install.sh`, `install.ps1` | Installers, so `releases/latest/download/install.sh` resolves |

Each OS builds and uploads its own archives with its own `.sha256` sidecar.
Release builds can run on three different machines without merging a shared
checksum file.

### 15.3 Release procedure

1. Bump the version in `Cargo.toml`, update the README if needed, and commit.
2. Tag `v<ver>` and push.
3. On each of Linux, macOS, and Windows: `make package && make publish`.
4. `make publish` creates the release on first use, uploads that machine's
   `dist/*` plus the installers, and replaces same-named assets on re-runs.
5. Run the installer one-liners on a clean machine of each OS.

The repository is public, so standard GitHub-hosted runners are free. CI runs `make ci` on all three OSes for every push and PR. Moving
step 3 into CI is a roadmap item; local builds remain fully supported.

### 15.4 `install.sh` (Linux, macOS)

```sh
curl -fsSL https://github.com/nealhardesty/pawprint/releases/latest/download/install.sh | sh
curl -fsSL .../install.sh | sh -s -- --version 1.2.0 --no-autostart
```

Steps: detect OS and arch → resolve version (`latest` by default) → download
the archive and its `.sha256` → verify (`sha256sum` or `shasum -a 256`) →
install to `~/.local/bin/pawprint` → `pawprint autostart on` → start the
badge → print `pawprint explain` and a PATH hint if `~/.local/bin` is not on
`PATH`.

Flags: `--version V`, `--archive PATH` (install from a local archive, with no
network access), `--prefix DIR`, `--no-autostart`, `--no-start`,
`--uninstall`, `--purge` (with uninstall, also delete config and state).
Environment equivalents: `PAWPRINT_VERSION`, `PAWPRINT_PREFIX`. The
repository is public, so downloads need no credentials.

The script is POSIX `sh` (no bashisms), uses only `curl` or `wget`, `tar`, and
a SHA-256 tool, and changes nothing until verification passes.

### 15.5 `install.ps1` (Windows)

```powershell
irm https://github.com/nealhardesty/pawprint/releases/latest/download/install.ps1 | iex
& ([scriptblock]::Create((irm https://github.com/nealhardesty/pawprint/releases/latest/download/install.ps1))) -Version 1.2.0 -NoAutostart
```

Same steps and flags as `install.sh` (`-Version`, `-Archive`, `-NoAutostart`,
`-NoStart`, `-Uninstall`, `-Purge`). Installs to
`%LOCALAPPDATA%\Programs\pawprint\`, adds that directory to the **user**
`PATH`, runs `Unblock-File` on the binaries, and verifies with
`Get-FileHash -Algorithm SHA256`. It works on Windows PowerShell 5.1 and
PowerShell 7. Piping into `iex` is not subject to the script execution
policy.

### 15.6 Upgrades and uninstall

Re-running the installer upgrades in place: it stops the running badge
(`pawprint quit`), replaces the binary, and starts it again. Config, state,
and user mascots are in separate directories and are never touched except by
`--purge`.

---

## 16. Testing and acceptance

### 16.1 Automated (`make test`, every OS in CI)

- **Hostname pipeline:** table-driven tests for `short`, labels, truncation,
  and case, including `owl.lab.example`, `OWL-02.LAB.EXAMPLE`, `prod-owl02`,
  `snowless`, `red-panda-01`, `bear-moose` (ambiguous → fallback), `x`, and
  63-character names.
- **Hash:** frozen vectors from [5.3](#53-choosing-a-mascot), plus a
  distribution check (10,000 synthetic hostnames over 100 mascots, no mascot
  above 3× its expected share).
- **Catalog stability:** a golden list of `(hostname → mascot)` for 200
  synthetic names under the bundled catalog. A change here means fallback
  assignments moved on fresh installs, which requires a conscious update.
- **Layout:** anchor/offset ↔ rectangle round-trips, clamping, the 3 × 3
  snap, and nameplate placement at each size.
- **Config:** precedence, globs, unknown-key warnings, and invalid-value
  fallback.
- **Golden renders:** badge renders for a fixed set of (mascot, size,
  label, frame), using the same `pawprint_core::render::render_badge` as
  `pawprint render`, are compared with `tests/golden/` as decoded RGBA
  pixels (not PNG bytes, so a PNG encoder update can't cause a false
  failure). `make golden` rewrites them.
- **Assets:** [12.11](#1211-validation-rules) on the full catalog.
- **Sprite compiler:** layer ops, bases and recolors, templates, props, and
  PNG import/export round-trips.
- **Shared move mode and layout** driven through `FakeSurface`.
- **Shell pieces:** backend selection (§3.2), scale helpers, the IPC
  protocol and a real socket round trip, session keys, log rotation, and the
  Wayland keymap reader.

#### 16.1.1 Display-server tests (Linux)

`make test-x11` and `make test-wayland` run the real `pawprint run` against
real display servers and check the screen. They are `#[ignore]`d in
`make test` because they need the servers installed; a separate Linux CI
job installs them (`xvfb xcompmgr openbox sway swaybg grim foot wtype dbus`) and
runs `make test-linux-display`. Each test gets its own HOME and runtime
directory and fails with the badge's log.

| Suite | Session | What is asserted |
|---|---|---|
| X11 | Xvfb, no WM, no compositor | override-redirect, depth 24 (shaped); DOCK type and ABOVE/STICKY/SKIP_* state; screen pixels equal `pawprint render` on opaque pixels and the backdrop on transparent ones; an XTest click on the label reaches the window below; in move mode a drag moves the badge, saves the snapped anchor and offset, and is pixel-exact at the new spot; arrow keys, Shift, Enter, and Escape; a `_NET_WORKAREA` change and an `Xft.dpi` change re-lay it out |
| X11 | Xvfb + `xcompmgr` started and stopped under a running badge | the window is rebuilt as 32-bit ARGB and back; pixel-exact and click-through in both |
| X11 | Xvfb + `openbox` | a managed DOCK, at the requested position, above a normal window mapped later, clicks pass through to that window (F32), drag works |
| X11 | GNOME-like environment (`XDG_CURRENT_DESKTOP=GNOME`, no layer-shell) | the `gnome` profile is chosen, ARGB, keyed on `WAYLAND_DISPLAY` |
| X11 | Xvfb | single instance: a second `run` and bare `pawprint` say "already running"; after a crash the stale socket is replaced; bare `pawprint` detaches; `quit` removes the socket |
| Wayland | headless sway, pixman renderer, `swaybg` backdrop | layer surface pixel-exact at top-right (via `grim`); a virtual-pointer click on the badge focuses the `foot` window under it; in move mode the click is taken |
| Wayland | headless sway | move-mode drag far outside the badge (F7) saves the right position and is pixel-exact; `wtype` arrow keys, Enter, Escape with exclusive keyboard |
| Wayland | headless sway + `swaybar` | the badge sits below the bar's exclusive zone (F3), pixel-exact |
| Wayland | headless sway | output mode change; scale 2 crisp (every opaque pixel exact); scale 1.5 sized 96 px and placed right (crispness at fractional scales is an owner check, F34) |
| X11 | Xvfb, move mode | the dashed accent outline appears on the badge edge (a dash in the transparent corner, art through a gap) and goes when move mode ends |
| X11 | Xvfb + private `dbus-daemon` + a stand-in StatusNotifierWatcher (the test's own zbus object) | the item registers with `Id`, `Title`, `ToolTip`, and a 32 and 64 px `IconPixmap`; the dbusmenu layout has the shared menu with the right checks; **Size ▸ 128** (a dbusmenu `Event`) resizes the badge and appends `[hosts.owl] size = 128` after the untouched `config init` template; **Animation ▸ Static** is saved; `set`/`unset` reload the running badge; an unknown mascot is refused (exit 2) without writing; a hand edit is picked up at the next IPC request (mtime); `tray = false` removes the item and `unset tray` brings a new one; **Quit** exits; the saved choice survives the restart |
| X11 | Xvfb | `pawprint preview owl`: a normal window (not override-redirect, no DOCK type, `WM_DELETE_WINDOW`), centered, the badge pixel-exact on the backdrop, the frame changes as actions play; Esc closes it, and so does a `WM_DELETE_WINDOW` message |
| Wayland | headless sway | `pawprint preview owl`: an xdg toplevel (`app_id` `pawprint`) that sway floats (fixed size), focused, the badge pixel-exact on the backdrop; Esc closes it |

What these can't show stays in the manual checks below and the facts ledger:
real Plasma and GNOME, real input devices, and remote clients.

#### 16.1.2 GUI-session test (macOS)

`make test-macos-display` runs the real `pawprint run` in the logged-in
desktop (a CI job runs it on `macos-latest`, whose job runs inside an
auto-logged-in GUI session, FM3). It is `#[ignore]`d in `make test` and
uses its own HOME and `TMPDIR`. It asks the window server
(`CGWindowListCopyWindowInfo`) about the badge's window and asserts:
status level (layer 25), the frame `status` reports, read-only sharing
(F12), an Accessory app that is never frontmost or active (also in move
mode), one badge per session, `status`/`move`/`reload`/`reset-position`/
`quit` over the socket, a clean exit that removes the socket, and that
the SystemConfiguration hostname equals `scutil --get`. With Screen
Recording permission it compares every opaque badge pixel in a
`screencapture` with `pawprint render` (scale 1 only); with Accessibility
permission it drags the badge with posted mouse events and checks the
saved, snapped position (and that it survives a restart), an arrow nudge,
Enter, and Escape. Without a permission, that check prints `SKIP` and why.
Click-through, Spaces, full-screen apps, and Retina scales stay owner
checks ([16.2](#162-backend-smoke-checks-manual-per-backend-every-release)).

#### 16.1.3 Desktop test (Windows)

`make test-windows-display` runs `pawprintw.exe run --verbose` (stderr to a
file) on the current desktop and checks it with Win32 calls from the test
(`crates/pawprint/tests/windows.rs`). The test's own windows run on a thread
that keeps dispatching messages, so they never stall the shell's
synchronous broadcasts, and a watchdog fails the test after two minutes. It is `#[ignore]`d in `make test`; a
`windows-latest` CI job runs it. It refuses to start while a badge runs in
the session, and restores the user's state file afterwards.
Win32 known folders can't be redirected, so it uses the real profile.

| Check | How |
|---|---|
| Styles | `GetWindowLongPtrW`: `WS_POPUP`; `LAYERED`, `TRANSPARENT`, `TOPMOST`, `TOOLWINDOW`, `NOACTIVATE`; no `APPWINDOW`; never the foreground window |
| Placement | top-right of the primary work area, 16 logical px in; `GetWindowRect` equals `status` |
| Topmost and pixels | a normal solid window shown *after* the badge covers the work area; a `BitBlt` with `CAPTUREBLT` of the badge equals `pawprint render` on opaque pixels and the backdrop on transparent ones (at scale 1) |
| Click-through | `WindowFromPoint` skips the badge, and a `SendInput` click on the label reaches the window below |
| Move mode | `pawprint move` clears `TRANSPARENT`; `WindowFromPoint` finds the badge on the art and the backdrop on a transparent pixel; a `SendInput` drag moves it, saves the position, and is pixel-exact; arrow keys and Enter when Windows grants the foreground |
| Work area | an app bar docked at the top (40 px) moves the badge down 40 px, and back when it goes (FW5) |
| IPC and instances | `status`, `move`, `reset-position`, `quit` over the pipe; a second `run` says "already running"; bare `pawprint` starts `pawprintw.exe` detached |

Pixel, click, drag, and key checks are skipped, with a message, when the
session has no visible desktop (the capture shows one flat color).

### 16.2 Backend smoke checks (manual, per backend, every release)

1. Badge visible, transparent, top-right, correct label and mascot.
2. Clicks and scrolling pass through to the window underneath.
3. Never takes focus. Absent from the taskbar, Dock, Alt-Tab, and pager.
4. Present on every virtual desktop or Space.
5. Move mode: drag, release, persists across `pawprint quit` and restart.
   Escape cancels. Times out after 30 s.
6. Resolution, scale, and monitor changes keep the anchor and clamp.
7. Static mode: no CPU use over 5 minutes (Task Manager, Activity Monitor, or
   `top`).

### 16.3 Spike exit criteria (M1 one-owl prototype)

On Windows, macOS, X11, Plasma Wayland, and Ubuntu GNOME Wayland: one owl,
short hostname label on the primary display, above normal windows,
click-through, and position remembered after a move-mode drag. For GNOME,
also: crisp at 100% and 200%, and crisp at 125% with
`xwayland-native-scaling` enabled. Failing any GNOME check triggers Plan B
([11.5.3](#1153-plan-b-the-presenter-extension)).

### 16.4 Remote compatibility matrix

The project owner runs this before each release. Every cell gets four checks:
**V**isible in the stream, **C**licks pass through, **M**ove mode works,
**P**osition survives reconnect or resize.

| Client → / Host ↓ | RDP | Jump Desktop | VNC | Parsec | Apollo/Moonlight |
|---|---|---|---|---|---|
| Windows 11 | ☐ V C M P | ☐ | ☐ | ☐ | ☐ |
| macOS | n/a | ☐ | ☐ | ☐ | n/a |
| Linux X11 (xrdp / x11vnc / Xvnc) | ☐ | ☐ | ☐ | n/a | n/a |
| Linux Plasma Wayland (KRdp / krfb) | ☐ | ☐ | ☐ | n/a | n/a |
| Ubuntu GNOME Wayland (GNOME Remote Desktop: sharing and remote login) | ☐ | ☐ | ☐ | n/a | n/a |

Cells marked n/a are combinations the client doesn't offer. Record results
in the release notes.

---

## 17. Accounts and external services

| Service | Used for | Required for |
|---|---|---|
| GitHub repository `nealhardesty/pawprint` | Source, issues, releases, CI | Public. Maintainers: `make publish` needs `gh auth login` with `repo` scope |
| GitHub Actions (hosted runners) | `make ci` on Linux, macOS, Windows | Nothing locally |
| Apple Developer ID / Windows code signing | — | Not used at 1.0 (roadmap) |

PAWPRINT itself contacts **no** service. The installers contact only
`github.com` and its release asset CDN.

---

## 18. Facts ledger

Hard-won facts, with their status. **Update this table when a spike confirms
or refutes an entry.** Never delete a refuted fact; mark it and say what is
true instead.

| # | Fact | Consequence | Status |
|---|---|---|---|
| F1 | `winit`'s `WindowLevel::AlwaysOnTop` is unsupported on Wayland, and winit has no layer-shell. | Own backends; layer-shell on Wayland | Documented |
| F2 | GNOME/Mutter does not implement `zwlr_layer_shell_v1`; Plasma (KWin) and wlroots compositors do. | GNOME supported through XWayland (11.5) | Documented |
| F3 | Layer-shell `exclusive_zone = 0` places a surface clear of other surfaces' exclusive zones (panels). An unmapped, four-edge-anchored 0 × 0 surface is configured to the usable area's size. | Margins are relative to the usable area; work-area probes | **Confirmed on sway 1.9** (`make test-wayland`, swaybar at the top); verify on Plasma |
| F4 | An empty `wl_surface` input region makes the surface click-through. | Wayland click-through | **Confirmed on sway 1.9** (a click on the badge focuses the window under it); verify on Plasma |
| F5 | A layer surface's output can't change after creation. | Recreate the surface on display change | Documented |
| F6 | Wayland has no core "primary output"; Plasma exposes order via `kde_output_order_v1`. | Primary detection on Plasma | Documented (verify) |
| F7 | Wayland implicit pointer grab delivers motion outside the surface while a button is held. **But** sway 1.9 reports those positions relative to where the surface was at the press, not where it is after it moves, so dragging the badge surface by its own pointer events runs away. | Move mode uses a still, full-area catcher surface instead (11.4) | **Refuted as a drag method on sway 1.9** (motion outside: confirmed; coordinates: press-relative). Catcher drag confirmed on sway; verify on Plasma |
| F8 | `WS_EX_LAYERED | WS_EX_TRANSPARENT` makes a window click-through; `UpdateLayeredWindow` needs premultiplied BGRA. | Windows backend | **Confirmed on windows-latest** (`make test-windows-display`: a `SendInput` click on the badge reaches the window below; the screen equals `pawprint render` pixel for pixel; see FW1–FW2) |
| F9 | `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)` hides a window from capture-based remote tools. | Never call it | Documented |
| F10 | `Local\` kernel objects and HKCU Run entries are per logon session; RDP sessions are separate sessions. | Per-session instance; autostart covers RDP | Documented |
| F11 | GUI-subsystem Windows exes have no usable stdout; console exes flash a window at logon. | Ship `pawprint.exe` + `pawprintw.exe` | Documented. Refined by FW4: a GUI-subsystem process does write to stdout/stderr handles it *inherits* (redirected to a file or pipe); it has no console of its own |
| F12 | macOS windows default to `sharingType = .readOnly`; `.none` hides them from capture. | Leave the default | **Confirmed** for the default (macOS 26.7 on `macos-latest`: the panel's `kCGWindowSharingState` is 1, read-only, and `screencapture` sees it pixel-exact); `.none` hiding it is documented |
| F13 | macOS `gethostname()` can reflect a network-assigned name; `LocalHostName` is stable. | Hostname source order | Documented; the SystemConfiguration read equals `scutil --get` (FM4) |
| F14 | `curl` downloads don't get `com.apple.quarantine`; browser downloads do. | curl installer avoids Gatekeeper prompts | Documented (verify) |
| F15 | Apple Silicon requires signed arm64 code; ad-hoc signing suffices for local execution. | Re-sign after `lipo` | **Verify** |
| F16 | `LimitLoadToSessionType = Aqua` restricts a LaunchAgent to GUI sessions. | No badge attempts in SSH sessions | Documented |
| F17 | X11 ARGB transparency needs a compositing manager (`_NET_WM_CM_S<n>` owner). Without one, the Shape extension's bounding mask gives 1-bit transparency. | Binary alpha + shape fallback | **Confirmed** (Xvfb with and without `xcompmgr`, switched live; both pixel-identical to `pawprint render`) |
| F18 | xrdp and Xvnc sessions are X11 even when the console runs Wayland. | X11 backend covers Linux RDP/VNC sessions | Documented |
| F19 | macOS ships GNU Make 3.81. | Makefile stays 3.81-compatible | Documented |
| F20 | GNU Make on Windows uses `sh.exe` if found on `PATH`, else `cmd.exe`. | Single-command recipes | Documented |
| F21 | Standard GitHub-hosted runners are free for public repositories. | CI on all three OSes | Documented |
| F22 | `x11rb`, pure-Rust `wayland-client`, `smithay-client-toolkit`, `calloop`, `zbus`, and `ksni` link into a fully static musl binary for x86_64 and aarch64 with only rustup pieces (`linker = "rust-lld"`, bundled musl CRT). x86_64: static-pie, 4.9 MB stripped, connects to X11 (XFixes 5.0, RandR), Wayland, and the session bus; exits cleanly with no session. aarch64: statically linked, 4.3 MB (not run; no qemu). Needs sctk without `xkbcommon` and wayland-client without `system`/`dlopen`/`log`. | One Linux binary for every distro; no cross toolchain for aarch64 | **Confirmed (M0 spike, `make spike-f22`)** |
| F23 | Whether an unowned topmost `WS_EX_TOOLWINDOW` window shows on all Windows virtual desktops is unconfirmed. | May need a workaround | **Verify** |
| F24 | Layer-shell overlays appear in Plasma's KRdp/krfb screencast streams. | Plasma Wayland remote support | **Verify** |
| F25 | Mutter keeps one stack for X11 and Wayland windows; an above-state XWayland window stays above normal Wayland windows. | GNOME stacking via XWayland | **Verify** |
| F26 | Mutter honors X11 input shapes on XWayland windows (GTK3 client-side-decoration shadows rely on it). | GNOME click-through | **Verify** |
| F27 | Xwayland's RandR primary output mirrors Mutter's primary monitor. | GNOME primary display | **Verify** |
| F28 | With fractional scaling, Mutter upscales XWayland windows (blurry) unless the `xwayland-native-scaling` experimental feature (GNOME 47+) is on. | `doctor` guidance; Plan B trigger | Documented (verify on Ubuntu) |
| F29 | Ubuntu enables the AppIndicator/KStatusNotifierItem extension by default; upstream GNOME has no tray. | Tray on Ubuntu; CLI-only on upstream GNOME | Documented |
| F30 | Premultiplied little-endian ARGB8888 (bytes B, G, R, A) is accepted by `UpdateLayeredWindow`, X11 32-bit ARGB visuals, `wl_shm` `ARGB8888`, and `CGImage` (premultiplied-first, 32-bit little-endian). | One pixel format for every backend | Documented |
| F31 | GNOME sessions set `DISPLAY` and start Xwayland on demand, including releases without an X11 session. | GNOME path needs nothing installed | Documented (verify on Ubuntu 26.04) |
| F32 | Openbox reparents even `_NET_WM_WINDOW_TYPE_DOCK` windows into a frame and copies the client's bounding shape to it, but not its input shape, so the frame swallows clicks. | The X11 backend sets its input shape on its frame too (11.3.1) | **Confirmed** (openbox 3.6 under Xvfb) |
| F33 | `x11rb` 0.14 connects through the `/tmp/.X11-unix` socket or TCP, not the abstract socket namespace. Where `/tmp/.X11-unix` is read-only (WSLg), a test Xvfb listens on abstract sockets only. | Tests run Xvfb with `-listen tcp`; real X servers always create the filesystem socket | **Confirmed** |
| F34 | Headless sway 1.9 with the pixman renderer filters the whole output at fractional scales (even swaybg's solid color comes out uneven); about 85% of the badge's pixels at 1.5 were exact. At integer scales every pixel is exact. | Fractional-scale crispness can't be judged there | **Confirmed on headless sway**; crispness at 125%/150% is an owner check on Plasma |
| F35 | wlroots `keyboard_interactivity = exclusive` on an overlay layer surface sends it the keys at once; virtual keyboards (`wtype`, remote-desktop servers) send keycodes from their own keymaps. | Exclusive keyboard in move mode; keycodes read from the keymap (11.4) | **Confirmed on sway 1.9**; verify on Plasma |
| F36 | zbus 5 (async-io) runs its socket setup (peer credentials, the SASL zero byte) on the `blocking` crate's thread pool. From `blocking` 1.7 an idle pool thread never exits (it exits only above the thread limit) and wakes every 500 ms; 1.6 lets it exit after 500 ms idle. | `blocking` is pinned to `=1.6.2`, so the badge has zero wakeups at rest with the tray running (7.2) | **Confirmed** (`make power-check`: 39 wakeups in 20 s in static mode with 1.7.0, 0 with 1.6.2) |
| F37 | `toml_edit` keeps a document's trailing comments after its last table, so a table added to a file that ends in comments lands above them. | The `config init` template ends with an empty `[hosts]`, so host tables go after the commented examples (8.6) | **Confirmed** (`config_edit` tests) |
| F38 | A `ksni` item with `MENU_ON_ACTIVATE` and the dbusmenu layout can be driven entirely over D-Bus: a stand-in StatusNotifierWatcher on a private `dbus-daemon` receives the registration, and a dbusmenu `Event(id, "clicked")` runs the item's callback. | The tray is tested end to end in `make test-x11` without a desktop (16.1.1) | **Confirmed**; real hosts (Plasma, Ubuntu's AppIndicator) are owner checks |

**macOS facts (M1 macOS backend).** Learned on GitHub's `macos-latest`
runner (macOS 26.7.2, Apple Silicon, one 1024 × 768 "Apple Virtual"
display at scale 1, auto-logged-in GUI session) by `make test-macos-display`.

| # | Fact | Consequence | Status |
|---|---|---|---|
| FM1 | The `CGImage` part of F30 holds: premultiplied BGRA bytes as `kCGImageAlphaPremultipliedFirst \| kCGImageByteOrder32Little`, tagged with the screen's color space, on a nearest-filter layer, reach the screen unchanged. | One pixel format on macOS too | **Confirmed** (every opaque pixel of the badge in a `screencapture` equals `pawprint render`) |
| FM2 | A borderless non-activating `NSPanel` at `NSStatusWindowLevel` in an Accessory app shows as window layer 25 at exactly the requested frame, the app has `activationPolicy = accessory`, and neither showing it nor `makeKeyWindow` in move mode makes it frontmost or active. | Contract items 2–4 on macOS (11.2) | **Confirmed** on the runner; Spaces, full-screen apps, and Mission Control are owner checks |
| FM3 | GitHub's macOS runner job runs inside the auto-logged-in GUI login: `SessionGetInfo` reports an audit session with window server access, `screencapture` captures other apps' windows, and `CGPreflightPostEventAccess` is true, so synthetic drags and keys reach the panel. | The backend is tested end to end in CI (11.2, 16.1) | **Confirmed** |
| FM4 | `HostName` from the SystemConfiguration preferences path `/System/System`, else `SCDynamicStoreCopyLocalHostName`, equals `scutil --get HostName`/`LocalHostName`. (The runner's is a generated `sat12-…-FE304CC6C67E.local`.) | No `scutil` subprocess (5.1) | **Confirmed** |
| FM5 | Posted mouse events reach a panel that just stopped ignoring the mouse; drags keep arriving outside the panel while the button is held, and `NSEvent.mouseLocation` tracks them in global points. | Move-mode drag without a pointer grab (11.2) | **Confirmed** on the runner |

**Windows facts (M1 Windows backend).** Learned on GitHub's `windows-latest` runner
(Windows Server, an interactive desktop at 1024 × 768, scale 1, the taskbar
taking the bottom 48 px) through `make test-windows-display`.

| # | Fact | Consequence | Status |
|---|---|---|---|
| FW1 | A layered window drawn with `UpdateLayeredWindow` hit-tests per pixel: fully transparent pixels pass the pointer through even without `WS_EX_TRANSPARENT`. | In move mode the drag starts on the mascot or the crate, not on the empty corners (11.1.4) | **Confirmed** (`WindowFromPoint` finds the window below on a transparent pixel in move mode) |
| FW2 | `BitBlt` from the screen DC includes layered windows only with `CAPTUREBLT`. | The test captures with `SRCCOPY | CAPTUREBLT` | **Confirmed** |
| FW3 | A normal window shown and activated after the badge stays below it (`WS_EX_TOPMOST`), and a `WS_EX_NOACTIVATE` window is never made foreground by showing, placing, or redrawing it. | Contract items 2 and 3 | **Confirmed** |
| FW4 | A GUI-subsystem exe writes to inherited stdout/stderr: `pawprintw.exe run --verbose` with stderr redirected to a file logs there. | The test reads the badge's log from its stderr file | **Confirmed** |
| FW5 | `SPI_SETWORKAREA` from an ordinary process returns success but leaves the work area unchanged while Explorer runs. An app bar (`SHAppBarMessage` `ABM_NEW` + `ABM_SETPOS`) does shrink it, and the shell then broadcasts `WM_SETTINGCHANGE` with `SPI_SETWORKAREA`. | The test simulates a taskbar with an app bar | **Confirmed** (the badge re-lays out to the 40 px smaller work area and back) |
| FW6 | `AllowSetForegroundWindow(ASFW_ANY)` from the process running `pawprint move`, then `SetForegroundWindow` in the badge, gives the badge the keyboard when that process may set the foreground. | Arrow-key nudging works from a terminal (11.1.4) | **Confirmed on the runner**; verify from Windows Terminal and conhost on a desktop |
| FW7 | `CreateNamedPipeW` with `FILE_FLAG_FIRST_PIPE_INSTANCE` fails with `ERROR_ACCESS_DENIED` when the name exists, and a pipe name vanishes when its last server handle closes. | The pipe is the instance lock; no stale endpoints (10.4) | **Confirmed** (`make test` on windows-latest; the desktop test's crash-free restarts) |
| FW8 | `std::process::Command` on Windows creates children with handle inheritance on, so a child inherits every inheritable handle, including the parent's own redirected stdout and stderr, even when its stdio is set to null. | Bare `pawprint` marks its stdio handles non-inheritable before starting the detached badge; otherwise `pawprint \| more` or a script capturing its output would wait until the badge quits | **Confirmed** (the desktop test hung until this was fixed) |

---

## Appendix A: launch catalog

The planned 100 mascots, in six packs. Base assignments are in
[12.8](#128-bases). ★ marks the **showcase set** (the 12
`required` IDs). Aliases listed are in addition to the ID itself. Accents are
from [Appendix B](#appendix-b-accent-palette). The signature action is the art
brief for each mascot's main idle action.

### Mammals (32)

| ID | Extra aliases | Accent | Signature action |
|---|---|---|---|
| ★ `moose` | — | pine | Heavy nod with broad paddle antlers |
| ★ `elk` | wapiti | copper | Raises head, flicks an ear; tall branching antlers |
| ★ `marmot` | groundhog, woodchuck | ochre | Pops upright, sniffs twice, sinks back |
| ★ `armadillo` | dillo | slate | Curls into a ball, rolls half a turn, peeks out |
| ★ `bear` | grizzly, brownbear | cranberry | Tiny yawn, one paw over the crate edge |
| ★ `chipmunk` | chippy | marigold | Cheeks puff as it stashes a seed |
| `fox` | redfox | brick | Ears swivel, tail swishes once |
| `capybara` | capy | honey | Eyes half-close in deep calm; a leaf lands on its head |
| `raccoon` | trashpanda | charcoal | Washes its paws, glances around |
| `red-panda` | firefox, lesserpanda | brick | Stands up tall, arms raised |
| `otter` | — | sky | Holds a pebble, juggles it once |
| `badger` | — | charcoal | Digs, flicking two crumbs out |
| `beaver` | — | copper | Gnaws a stick, tail slaps |
| `hedgehog` | hedgie | ochre | Puffs its spines, deflates |
| `rabbit` | bunny, hare | rose | Nose wiggle, one ear flops |
| `squirrel` | — | copper | Tail flicks twice, freezes |
| `mouse` | mice | slate | Whiskers twitch, nibbles a crumb |
| `hamster` | — | honey | Stuffs cheeks, runs in place |
| `deer` | doe, fawn | moss | Head lifts, ears forward, alert blink |
| `bison` | buffalo | ochre | Snort with a puff of breath |
| `wolf` | — | slate | Quiet howl, head tilted up |
| `opossum` | possum | charcoal | Plays dead, then peeks with one eye |
| `skunk` | — | violet | Tail rises, a tiny stink cloud |
| `sloth` | — | moss | Ever-so-slow blink and smile |
| `koala` | — | sky | Hugs the crate edge, dozes |
| `llama` | alpaca | rose | Chews, side-eye, ears twitch |
| `panda` | giantpanda | charcoal | Munches bamboo, rolls back |
| `lion` | — | marigold | Mane shake, small roar |
| `tiger` | — | marigold | Stretch with a big yawn |
| `monkey` | ape, chimp | copper | Scratches head, grins |
| `elephant` | — | slate | Trunk curls up, trumpets |
| `goat` | — | moss | Chews, then a little head-butt |

### Cats and dogs (24)

| ID | Extra aliases | Accent | Signature action |
|---|---|---|---|
| ★ `cat-maine-coon-black` | mainecoon, maine | plum | Amber side-eye, one fluffy tail flick |
| ★ `cat-buff-tabby` | bufftabby, buff, chonk | honey | Slow blink, settles until only ears and cheeks show |
| ★ `cat-cow` | cowcat, moo | teal | Paws at a loose crate flap |
| ★ `cat-tuxedo` | tuxedo, tux | brick | Straightens up, tucks a white paw under its chest |
| `cat-calico` | calico | rose | Ear twitch, kneads the crate edge |
| `cat-siamese` | siamese | sky | Blue-eyed stare, silent meow |
| `cat-orange-tabby` | cat, kitty, orangecat, ginger | marigold | Yawn, then a blep |
| `cat-gray` | graycat, greycat, bluecat | slate | Tail curls, eyes squint |
| `cat-white` | whitecat | ice | Ears flatten at a noise, then relax |
| `cat-sphynx` | sphynx | rose | Wrinkly head tilt |
| `cat-tortoiseshell` | tortie, tortoiseshell | copper | Grooms a paw |
| `cat-black` | blackcat, void | violet | Only the eyes blink in the dark |
| `dog-corgi` | corgi | marigold | Ears perk, loaf wiggle |
| `dog-shiba` | shiba, doge | ochre | Side-eye smile, tail curl |
| `dog-greyhound` | greyhound, grey | sky | Long nose rests on the crate edge, sigh |
| `dog-pug` | pug | honey | Head tilt, snort |
| `dog-dachshund` | dachshund, wiener, doxie | copper | Long body slides further out, then back |
| `dog-labrador` | dog, lab, labrador | honey | Happy pant, tail wag |
| `dog-husky` | husky | ice | Awoo with its head thrown back |
| `dog-border-collie` | collie, bordercollie | teal | Alert crouch, eyes dart |
| `dog-beagle` | beagle | ochre | Sniffs the air, ears flap |
| `dog-poodle` | poodle | rose | Pom-pom bounce |
| `dog-chihuahua` | chihuahua | cranberry | Trembles, tiny bark |
| `dog-german-shepherd` | shepherd, gsd, alsatian | brick | Ears rotate, attentive head tilt |

### Birds (16)

| ID | Extra aliases | Accent | Signature action |
|---|---|---|---|
| ★ `owl` | owls, hoot | violet | One slow blink, then a sideways head turn |
| `crow` | raven, corvid | charcoal | Tilts its head, caws silently |
| `puffin` | — | brick | Bill clacks, little hop |
| `penguin` | — | ice | Waddle-shuffle, flipper flap |
| `kiwi` | — | moss | Long beak probes the crate |
| `heron` | egret | sky | Neck extends slowly, retracts |
| `parrot` | macaw, polly | cranberry | Head bob, feathers ruffle |
| `duck` | mallard | teal | Quack, tail waggle |
| `chicken` | hen, rooster | marigold | Pecks twice, head jerks |
| `flamingo` | — | rose | Lifts one leg, wobbles |
| `pelican` | — | ice | Pouch stretches in a yawn |
| `toucan` | — | marigold | Bill swings side to side |
| `eagle` | hawk, falcon | copper | Wings half-open, stern stare |
| `hummingbird` | hummer | teal | Hovers with blurred wings |
| `pigeon` | dove | slate | Coos with a chest puff |
| `swan` | goose | ice | Graceful neck curve, preen |

### Aquatic and cold-weather (12)

| ID | Extra aliases | Accent | Signature action |
|---|---|---|---|
| ★ `walrus` | — | ice | Whisker twitch, tusk-first head bob |
| `octopus` | octo, kraken | violet | Tentacle curls over the crate edge |
| `axolotl` | axo | rose | Gills flutter, smile |
| `narwhal` | — | sky | Tusk sparkles, gentle bob |
| `seal` | — | slate | Claps flippers |
| `whale` | orca | sky | Spout puff from the blowhole |
| `dolphin` | porpoise | teal | Leaps and dips |
| `shark` | — | slate | Fin circles, toothy grin |
| `crab` | — | brick | Claws snap, sidestep |
| `jellyfish` | jelly, medusa | violet | Pulses up, drifts down |
| `polar-bear` | — | ice | Sniffs the cold air, nose wiggle |
| `pufferfish` | puffer, fugu | marigold | Inflates, deflates |

### Reptiles, amphibians, and bugs (8) — category `critters`

| ID | Extra aliases | Accent | Signature action |
|---|---|---|---|
| `gecko` | lizard | moss | Licks its own eyeball |
| `frog` | toad | moss | Throat puffs, ribbit |
| `tortoise` | turtle | pine | Head retracts, slowly re-emerges |
| `moth` | — | honey | Wings flutter, fuzzy antennae |
| `beetle` | bug | pine | Shell opens a crack, wings peek |
| `snake` | python, viper | moss | Tongue flick, sway |
| `bee` | bumblebee, honeybee | marigold | Bobbing hover, buzz lines |
| `snail` | slug | ochre | Eye stalks extend and retract |

### Objects and oddballs (8) — category `oddballs`

| ID | Extra aliases | Accent | Signature action |
|---|---|---|---|
| `crt` | monitor, tv, terminal | charcoal | Screen flickers, prompt cursor blinks |
| `floppy` | disk, diskette | slate | Shutter slides open, closed |
| `mushroom` | shroom, fungus | cranberry | Cap bounces, spores puff |
| `ghost` | boo, spooky | ice | Floats up, fades a step |
| `ufo` | saucer | teal | Hovers, beam flickers |
| `robot` | bot, droid | slate | Antenna light blinks, head swivels |
| `cactus` | — | pine | A flower pops open |
| `teapot` | tea, kettle | rose | Lid rattles, steam puff |

Totals: 32 + 24 + 16 + 12 + 8 + 8 = **100**.

Alias collisions are resolved at catalog time. For example, `cat` and `dog`
belong only to `cat-orange-tabby` and `dog-labrador`, and `cow` is not an
alias of `cat-cow` (it stays free for a future `cow`).

---

## Appendix B: accent palette

Sixteen named accents. Order is significant (fallback index `0..15`) and
**frozen**: never reorder or remove. New named accents may be added after
`charcoal`, but the fallback formula always uses `mod 16` over these first
sixteen, so additions never move a fallback host's color.

| # | Name | Hex | # | Name | Hex |
|---|---|---|---|---|---|
| 0 | `violet` | `#7b5ea7` | 8 | `plum` | `#6e3b62` |
| 1 | `pine` | `#2f6b4f` | 9 | `honey` | `#d9a441` |
| 2 | `ochre` | `#c8962e` | 10 | `teal` | `#2a8c87` |
| 3 | `copper` | `#b8673a` | 11 | `brick` | `#a4462f` |
| 4 | `slate` | `#4f6d8f` | 12 | `moss` | `#6f7f3a` |
| 5 | `cranberry` | `#9e2b45` | 13 | `rose` | `#d77a91` |
| 6 | `ice` | `#8fc3d9` | 14 | `sky` | `#5b9bd5` |
| 7 | `marigold` | `#e8912d` | 15 | `charcoal` | `#4a4a4f` |

Each accent expands to a 3-step ramp (shadow, base, highlight) for crate
shading: HSL lightness −0.12 and +0.12, hue and saturation unchanged
(`pawprint_sprite::ramp`). `assets/accents.toml` stores only names and hex
values; the ramp is computed when drawing, the same way for bundled accents
and for `#rrggbb` accents from config.
