# PAWPRINT — Specification

This is the single source of truth for **how PAWPRINT works**: product rules,
architecture, formats, platform behavior, build and release mechanics, and the
facts we learned the hard way. Read it before changing anything. If code and
this document disagree, one of them is a bug; fix whichever is wrong in the
same change.

- What does not exist yet is in [`ROADMAP.md`](ROADMAP.md).
- How to install and use it is in [`README.md`](README.md).

**Status of this document:** everything described here is implemented
(except the conditional GNOME Plan B extension, 11.5.3) and tested in CI on
Linux, macOS, and Windows, including real display servers and desktops
([16.1](#161-automated-make-test-every-os-in-ci)) and the installers
([15.7](#157-installer-tests)). What remains is a published release and the
owner's checks on real hardware (Plasma, Ubuntu GNOME, remote clients); see
[`ROADMAP.md`](ROADMAP.md). The repository is public. Facts marked
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
   focus, never appears in the taskbar, Dock, or Alt-Tab, steps aside for
   fullscreen windows ([6.5](#65-fullscreen-windows)), and is moved only
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
```

**The badge is always exactly `size` × `size`** logical pixels (times the
display scale, rounded to whole art pixels: [4.3](#43-sizes-and-scaling)),
label included. Nothing is ever drawn outside that square: no nameplate, no
side label, no wider crate, no margin. The label always sits on the crate's
label strip and is fitted, then cut, to stay inside it
([4.4](#44-label)).

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
| Badge canvas | 32 × 32, always: the whole badge, label included |
| Mascot frame | 32 × 32, drawn at (0, 0) |
| Crate back rim | rows 18–19 |
| Crate front | rows 20–31 (12 rows), full width; the mascot is hidden behind it: outline (20), top rail (21), label strip, outline (31) |
| Label strip on crate front | rows 22–30, columns 2–29 (28 × 9 cream) |
| Label area | the strip less 1 art pixel on every side: 26 × 7 |
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
  1.5, 2.0, ...). The badge image is `32P × 32P` physical pixels.
- Scaling is always **nearest-neighbor by an integer factor**, so pixel art
  stays crisp. At fractional display scales the badge is a little larger or
  smaller than the nominal size. That is intended.

### 4.4 Label

**Text.** Derived from the hostname ([5.2](#52-labels)) or configured. A
label from the hostname keeps only its letters and digits (`prod-owl-02`
shows `PRODOWL02`); a label the user sets is shown exactly as written. Either
way it is cut to its **first 20 characters**. Characters outside printable
ASCII (0x20–0x7E) render as `?`.

**Fonts.** Two embedded bitmap fonts, both original to this repository and
covering printable ASCII:

| Font | Source | Glyph | Advance | Notes |
|---|---|---|---|---|
| Regular | `assets/font/font.toml` | 5 × 7 | 6 | True lowercase |
| Micro | `assets/font/micro.toml` | 3 × 5 | 4 | Lowercase uses the capital forms: 3 × 5 has no room for a legible lowercase, nor for small caps |

Both are validated the same way (`pawprint_sprite::Font::compile`): every
printable ASCII character present, nothing else, every glyph exactly the
font's size. In the micro font, digits and `-` `.` `_` are drawn first for
clarity, since hostnames are mostly those and letters: `0` is square and `O`
round, `8` is pinched and `B` is not, and `M`, `H`, and `W` differ by where
their crossbar sits (top, middle, bottom).

**Fit, then cut.** The label is drawn at **physical-pixel resolution** on
top of the scaled badge (`pawprint_core::layout::fit_label`), so bigger
badges show more and taller text. Its area is the strip less one art pixel
on every side: `26P × 7P` physical pixels.

1. **Candidates:** each font at every integer scale `f = 1, 2, 3, …` whose
   glyph height (`7f` or `5f`) fits the area's height and the `label_size`
   cap.
2. **Fit:** of the candidates whose full label width (`6n − 1` or
   `4n − 1` font pixels for `n` characters, times `f`) fits the area's
   width, the **tallest wins**; on a tie, the 5 × 7 font. The text is
   centered in the strip both ways (rounding down).
3. **Cut:** if no candidate fits the whole label, the micro font at `f = 1`
   draws as many **whole characters from the left** as fit, with no
   ellipsis, left-aligned at the area's edge and centered vertically. The
   left side of a name is what shows.

What each label gets (`label_size = "auto"`):

| Label | 32 (P = 1) | 64 (P = 2) | 128 (P = 4) | 128 at 200 % (P = 8) |
|---|---|---|---|---|
| `OWL` | 5×7 ×1 | 5×7 ×2 | 5×7 ×4 | 5×7 ×8 |
| `MOOSE`, `OWL02` | 3×5 ×1 | 3×5 ×2 | 3×5 ×5 | 3×5 ×10 |
| `ARMADILLO`, `PRODOWL02` | 3×5 ×1, cut to 6 (`PRODOW`) | 3×5 ×1 | 3×5 ×2 | 3×5 ×5 |
| `DESKTOP4F7Q2KL` (14) | 3×5 ×1, cut to 6 | 3×5 ×1, cut to 13 | 5×7 ×1 | 3×5 ×3 |
| `JENKINSAGENTLINUX07` (19) | 3×5 ×1, cut to 6 (`JENKIN`) | 3×5 ×1, cut to 13 | 3×5 ×1 | 3×5 ×2 |

At size 32, then, at most 6 characters show and at most 4 in the 5 × 7
font; at 64, 13 and 8; at 128, every 20-character label fits whole.

**Why the left side, and why letters and digits.** The owner's call: the
label must stay inside the crate at every size, may be cut, and must keep
its left-hand side; its height scales with the badge even where that makes
it less legible. This replaced 1.0's nameplate, a board under or beside the
crate that made the badge wider than `size` (up to 250 × 90 at size 64).
The cost is known: pool nodes that differ only in their suffix
(`gke-prod-default-pool-…-xyz9`, `jenkins-agent-linux-07` and `-08`) look
alike at small sizes. Fleets like that are better served by an explicit
`label` per `[hosts]` glob, or a larger `size`. Dropping punctuation from
hostname labels fits more of the name: a `-` costs a whole glyph cell, and
hostnames rarely need it to be read. User-set labels keep
theirs, because they were typed on purpose. Twenty characters still covers
every default hostname scheme measured in M4 (Windows `DESKTOP-4F7Q2KL`,
macOS `Johns-MacBook-Air`, AWS `ip-172-31-45-201`, Raspberry Pi); only
names of 21+ characters after stripping lose their end.

**Remote-client scaling.** A client showing a 4K desktop in a 1080p window
scales by 0.5. Text at `f = 1` is then below one screen pixel per font
pixel and not legible; `f = 2` and up survive. Users of scaled clients
should use `size = 128` or more and short labels.

**Size.** `label_size` caps the glyph height, in logical pixels times the
display scale; fitting and cutting work as above under the cap:

| `label_size` | Tallest glyph |
|---|---|
| `auto` (default) | no cap: the tallest that fits |
| `small` | 7 (5 × 7 at `f = 1`) |
| `medium` | 14 |
| `large` | 21 |

A cap never hides the label: if no candidate is under it, the micro font at
`f = 1` is used anyway.

**Contrast.** Dark text (`#1b1b1f`) on a light strip (`#f2efe6`) with the
accent around it. The strip and text colors are fixed for legibility. Only
the accent changes.

**Case.** `label_case = "upper"` (default) renders the label like a shipping
stamp. `"as-is"` keeps the configured or detected case. `"lower"` lowercases.
In the micro font both cases look the same.

### 4.5 Pixel rules

- Every color has alpha 0 or 255. No partial alpha in art or composition.
  This keeps edges crisp and makes the X11 shape fallback exact
  ([11.3.2](#1132-transparency-with-or-without-a-compositor)). It also makes
  premultiplied alpha trivial, which Windows and Wayland require.
- The renderer composes the badge into a small RGBA buffer at 1×, then
  scales it by `P` into the backend's buffer. A redraw happens only when the
  frame, label, size, or scale changes. `Badge` keys the composed image on
  everything it depends on (mascot, frame, label, label size, accent, size,
  scale, move-mode outline) and skips composition when the key
  matches; the tray icon is likewise composed only when the mascot or
  accent changes.

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
   - `"auto"` (default): `short`, letters and digits only (`prod-owl-02`
     becomes `prodowl02`)
   - `"full"`: `raw` lowercased, letters and digits only
     (`owl.lab.example` becomes `owllabexample`)
   - `"none"`: no label; the strip stays empty
   - any other string: used literally, punctuation and all
4. Apply `label_case`, then keep the first 20 characters. The strip may show
   fewer: it is fitted, then cut from the right ([4.4](#44-label)). An
   earlier rule cut long labels in the middle (`jenkins-a..-linux-07`) to
   keep the suffix; it was dropped because the strip itself now shows the
   left side, and the owner prefers the left side of a name.

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
- Because host names lose their digits before lookup, an alias or id that
  contains a digit (`r2d2`, `owl2`) can never match one. Such a mascot is
  still chosen by `mascot = "<id>"` or the fallback. Validation rejects
  digits in bundled ids and aliases and warns about them in user mascots
  ([12.11](#1211-validation-rules) rule 8).

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

### 6.5 Fullscreen windows

A fullscreen window on the badge's display (a remote-desktop client such
as `mstsc` or Jump Desktop in full screen, a video, a game) hides the
badge, and the badge comes back when that window leaves fullscreen or the
front. A local badge over a full-screen remote session would name the
wrong machine; the remote machine's own badge, inside the session, is the
right one. `fullscreen = "show"` keeps the badge on top instead (for a
kiosk that runs fullscreen all day).

The core sends the policy to the surface (`Surface::set_fullscreen`, from
the `fullscreen` setting, before the first `place` and on every change),
and each backend implements it the native way:

| Backend | `hide` (default) | `show` |
|---|---|---|
| Windows | An appbar registration (`ABM_NEW`, with no `ABM_SETPOS`, so no space is reserved) gets the shell's `ABN_FULLSCREENAPP`, and a `WINEVENT_OUTOFCONTEXT` hook gets `EVENT_SYSTEM_FOREGROUND`. Either one re-checks the foreground window; the badge is hidden (`SW_HIDE`) while it covers the badge's display, and shown again (`SW_SHOWNOACTIVATE`) after | Neither is registered |
| macOS | The panel leaves out `fullScreenAuxiliary`: a fullscreen app gets a Space of its own, and only auxiliary windows join it | `fullScreenAuxiliary` is set |
| X11 | The backend follows `_NET_ACTIVE_WINDOW` (on the root) and that window's `_NET_WM_STATE` and size (`PropertyNotify`, `ConfigureNotify` on it), and unmaps the badge while the active window is `_NET_WM_STATE_FULLSCREEN` over the badge's display | Nothing is watched |
| Wayland | The layer surface is on `top`, which compositors draw below fullscreen windows (sway, KWin) | `overlay`, above them |
| GNOME (X11 profile) | As X11 for XWayland windows. Native Wayland windows are invisible to X11 (`_NET_ACTIVE_WINDOW` is `None` then); Mutter itself puts DOCK windows in the bottom layer while their monitor is fullscreen (F40) | As X11, but Mutter may still lower the badge (F40) |

"Covers the badge's display" is decided once, in the core
(`surface::fullscreen_covers`): the window's rectangle contains the whole
display that the badge's center is on, so a fullscreen window on another
monitor leaves the badge alone, and one spanning every monitor (a
multi-monitor RDP session) hides it. On Windows a captioned window never
counts, so a maximized window under an auto-hidden taskbar doesn't, and
neither do the desktop (`Progman`, `WorkerW`) or PAWPRINT's own windows.
Nothing polls: every check is a reaction to a notification the system
already sends.

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

**Measured** with `make power-check` (Linux): the release build runs
under Xvfb with a private session bus (so the tray's threads count), first
in idle and then in static mode, 180 s each. CPU is `utime + stime` from
`/proc/<pid>/stat`; wakeups are the voluntary context switches of every
thread (`/proc/<pid>/task/*/status`); startup is process start until
`pawprint status` reports a placed badge, which the loop answers only
after the first frame is flushed. Startup is timed over five starts and
the fastest counts against the budget (the median is printed beside it):
a shared CI runner can only slow a start down, so one slow sample on a
busy machine says nothing about the badge. A single-sample check once
failed the macOS job at 411 ms on a queued runner whose other starts took
131–341 ms.

| Metric (Linux X11, M4) | Measured | Budget |
|---|---|---|
| Startup to visible | 16 ms (9 ms on a warm run) | < 300 ms |
| CPU, idle mode | 0.011% of a core | < 0.1% |
| Wakeups, idle mode | 49 in 180 s (each action's frames plus one per rest) | deadlines only |
| CPU, static mode | 0.000% | 0% |
| Wakeups, static mode | **0 in 180 s** | zero |
| Resident memory | 12.2 MB peak | < 30 MB |
| Binary size | 7.1 MB (glibc release build, 100 mascots); the shipped static musl build is 7.7 MB (x86_64) and 6.8 MB (aarch64), [14.5](#145-release-builds) | < 15 MB |
| Threads | 5 (loop, IPC, tray service, D-Bus reactor, and one idle) | — |

The first run found 39 wakeups in 20 s in static mode: an idle `blocking`
pool thread that zbus leaves behind and that, from `blocking` 1.7, never
exits (F36). Pinning `blocking` 1.6.2 removed them.

**Windows** (`make power-check` on Windows, run by the `windows-display` CI
job on `windows-latest`, 150 s per mode): the release `pawprintw.exe`
runs on the current desktop with its tray icon. CPU is `Get-Process`'s
`TotalProcessorTime`; wakeups are each thread's raw
`ContextSwitchesPersec` counter (`Win32_PerfRawData_PerfProc_Thread`,
counted per thread); memory is the working set; startup is the same
`pawprint status` poll. Win32 known folders can't be redirected, so it
swaps in its own `config.toml` and restores the user's config and state
files afterwards.

| Metric (Windows Server, `windows-latest`, M4) | Measured | Budget |
|---|---|---|
| Startup to visible | 38 ms | < 300 ms |
| CPU, idle mode | 0.000% of a core (below `TotalProcessorTime`'s 15.6 ms resolution over 150 s, so < 0.01%) | < 0.1% |
| Wakeups, idle mode | 50 in 150 s (each action's frames plus one per rest) | deadlines only |
| CPU, static mode | 0.000% | 0% |
| Wakeups, static mode | **0 in 150 s** | zero |
| Working set | 19.8 MB peak, 16.3 MB after 150 s | < 30 MB |
| Binary size | `pawprintw.exe` 4.5 MB, `pawprint.exe` 4.5 MB (release, 100 mascots) | < 15 MB |
| Threads | 2 (loop and IPC) | — |

The first Windows run summed every thread's counter and went backwards
when a short-lived loader thread exited between samples, so wakeups are
counted per thread id on Linux and Windows.

**macOS**: `make power-check` runs the universal release binary (`lipo` of
the arm64 and x86_64 builds) in the logged-in desktop with its own HOME and
TMPDIR, menu-bar item included, 150 s per mode in CI. Counters come from
`proc_pid_rusage` (what `top` and `powermetrics` read; no root): CPU is
user + system time (Mach units converted with `mach_timebase_info`),
wakeups are interrupt wakeups with package-idle wakeups in parentheses,
memory is the physical footprint (Activity Monitor's "Memory").

| Metric (macOS 26.6 on `macos-latest`, Apple Silicon VM, M4; three runs) | Measured | Budget |
|---|---|---|
| Startup to visible | 198, 240, 211 ms (cold CI runner) | < 300 ms |
| CPU, idle mode, whole process | 0.047%, 0.152%, 0.051% of a core | < 0.1% |
| CPU, idle over static (animating) | 0.016%, 0.076%, −0.011% of a core (inside the noise) | < 0.1% |
| Wakeups, idle mode | 383, 417, 379 in 150 s (75, 249, 119 package-idle) | deadlines only |
| CPU, static mode | 0.032%, 0.077%, 0.062% of a core (AppKit's threads) | 0% |
| Wakeups, static mode | 346, 368, 383 in 150 s (53, 266, 262 package-idle); **the badge's loop: 0** | zero |
| Memory footprint | 15.7, 16.8, 16.8 MB peak (resident size 45 to 54 MB, mostly shared framework pages) | < 30 MB |
| Binary size | 8.0 to 8.1 MB universal (arm64 + x86_64, 100 mascots) | < 15 MB |
| Threads | 4 to 5 | — |

In static mode the main thread sleeps in `nextEventMatchingMask` and
receives no event at all (each wakeup is logged at debug level, and
`make test-macos-display` asserts none in 15 s; FM7). The remaining
wakeups and CPU belong to AppKit's and the system's own threads in every
Cocoa process, varied from run to run (77 to 358 in 20 s, with or without
the menu-bar item), and the shared VM makes them swing between runs, so
`power-check` reports the whole-process static rows and idle CPU as
`over*` rather than failing on them, and judges **idle over static**, what
animating adds, against the idle budget.

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
label = "auto"             # "auto" (short hostname) | "full" | "none" | any text, shown as written
label_case = "upper"       # "upper" | "lower" | "as-is"
label_size = "auto"        # "auto" (tallest that fits) | "small" | "medium" | "large" (cap 7, 14, 21 px)

# Mascot size in logical pixels. 32, 64, and 128 are the presets; any multiple of 32 works.
size = 64

# Where the badge sits on the target display. Dragging in move mode overrides this.
position = "top-right"     # top-left | top | top-right | left | center | right | bottom-left | bottom | bottom-right
offset = [16, 16]          # logical px inward from the anchor (each -100000 to 100000)
display = "primary"        # "primary" | an output name from `pawprint doctor`

# Animation.
animation = "idle"         # "idle" | "static"
interval = [8, 20]         # seconds between idle actions (min, max), 1 <= min <= max <= 3600

# Crate color.
accent = "auto"            # "auto" | a palette name (see `pawprint list --accents`) | "#rrggbb"

# Tray icon (Windows), menu-bar item (macOS), or StatusNotifierItem (Linux).
tray = true

# When a fullscreen window covers the badge's display (a remote-desktop client,
# a video, a game): "hide" steps aside so the badge never shows over it (6.5).
fullscreen = "hide"        # "hide" | "show"

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
loads on older binaries. An invalid value (wrong type, out of range, or a
mascot or accent that isn't in the catalog) logs a warning and is ignored,
so that key falls back to the next layer down ([8.3](#83-precedence)): a
bad `mascot` in `[hosts.buildbox]` yields to a matching glob, then the
top level, then the default. A config file never prevents the badge from
starting.

### 8.3 Precedence

For each key, highest wins:

1. Command-line flags on `pawprint run` (debugging only)
2. `[hosts."<short>"]`: exact match on the short hostname
3. `[hosts."<glob>"]`: matching globs, most specific first (longest pattern,
   then fewest wildcards, then file order)
4. Top-level keys
5. Built-in defaults

Host keys compare case-insensitively. A key with a dot is matched by its
part before the first dot, the way the short name is made
([5.2](#52-labels)): `[hosts."web01.example.com"]` applies to `web01`, and
`"web*.example.com"` acts as `web*` (so `"*.example.com"` matches every
host). If several exact tables match (`[hosts.OWL]` and `[hosts.owl]`), the
last one in the file applies, and `pawprint set` edits that one.

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

The temporary file is flushed to disk (`sync_all`) before the rename. A bad
entry (an unknown key, an invalid `[position]`, an assignment that isn't a
mascot id) is dropped with a warning and the rest is kept. A file that can't
be used at all (not TOML, or no supported `version`) loads as an empty
state with a warning, and the next save first copies it to
`state.toml.bad`, so nothing is lost when it is overwritten.

### 8.5 Reloading

The running badge reloads its config and its user mascots
([12.7](#127-png-bridges)) when `pawprint reload` is run, when the tray
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
  (the existing exact table loading uses, matched as in 8.3, else a new
  one, appended at the end). An exact host table outranks globs and top-level
  keys (8.3), so the choice always takes effect, and a config copied to
  another machine doesn't carry it over. Only `pawprint run` flags rank
  higher.
- **How:** with `toml_edit`, so comments, order, and formatting stay. A
  value already in the table keeps its trailing comment. `unset` removes
  the key, then the table if it is left empty without comments, then an
  empty implicit `[hosts]`. A file that isn't valid TOML is left alone and
  the command fails. The write is atomic (temporary file, then rename),
  keeps the file's permissions, and goes to a symlink's target, so dotfile
  managers keep their link. A symlink to a missing file is an error, never
  silently replaced by a regular file.
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

### 8.7 The settings window

`pawprint settings`, or the tray's **Settings…**, opens a window that sets
**every key in 8.2**, in **any table**: this host (the default, the same
table `pawprint set` uses), all hosts (the top level), or any
`[hosts."<pattern>"]` table, which it can also add, rename, and delete. It
is not a text editor for the file: each key has its own control, and
every change goes through `config_edit` (`set_in`, `unset_in`,
`add_host`, `rename_host`, `remove_host` with a `Scope`), so the file keeps
its comments, order, and formatting, and values are checked by the loader's
own parser (`config::check_value`) before anything is written.

- **Look.** Pixel art, drawn by the core (`pawprint_core::settings`, on
  the `pawprint_core::ui` canvas) in the bundled fonts: 360 × 252 units,
  scaled by a whole number (2 per unit at scale 1, 4 at scale 2, smaller
  if the display's usable area is too small) and centered on the primary
  display. A title strip with the config path; a scope row ("changes
  apply to"); pages on the left (Mascot, Label, Size+color, Position,
  Animation, System, Host tables); the badge as this host will show it
  below the pages; a status line with Undo and Close.
- **Controls.** Mascot: Auto and a scrollable grid of every mascot (user
  mascots included) by category; typing searches by id, name, and alias,
  Enter picks the first match, Esc clears the search. Label: Auto, Full,
  None, or typed text; letter case; text size. Size: 32, 64, 128, or any
  multiple of 32 with − and +. Crate color: Auto, the 16 palette swatches,
  or a typed `#rrggbb`. Position: a 3 × 3 grid, the offset's X and Y, the
  display (Primary, each connected display, or a typed name). A dragged
  position (8.4) is shown, since it wins over these, with **Forget the
  drag** (the running badge's `reset-position`, else the state file).
  Animation: Idle or Static, and the interval's two ends, kept min ≤ max.
  System: tray, fullscreen (6.5), and backend.
- **Inheritance.** Each setting shows where its value comes from: set in
  this table (a filled chip, with **Clear**) or inherited (an outlined
  chip, "from all hosts", "from [hosts.prod-*]", "default"). What a table
  inherits is what applies below it for this host: an exact host table
  inherits from matching globs and the top level, a glob table from the
  top level, the top level from the defaults. A value set in a table but
  overridden for this host by a stronger one says so in red ("owl uses
  128 from [hosts.o*]"). Clearing a key in this host's table removes the
  emptied table, as `pawprint unset` does; other tables stay until
  deleted on the Host tables page.
- **Saving.** There is no Save button: every change is written at once
  (atomically, through the same `commands::write_config` as `set`), then
  the running badge is told to reload, so the badge on the desktop is the
  live preview. Hovering a choice previews it in the window's own badge
  without writing. Undo (and Ctrl+Z, Command+Z on macOS) writes the
  previous text back, through every change made in the window. A failed
  write puts the last good text back and says why.
- **Text fields** commit on Enter, Tab, or a click elsewhere; Esc cancels.
  Invalid input (a size that isn't a multiple of 32, an unknown color) is
  refused with the loader's reason and nothing is written.
- **A file that isn't valid TOML** is shown, not edited: the window says
  so and shows the parser's message. When the file changes on disk
  (another editor, `pawprint set`), the window shows the new version at
  its next wakeup (the same one-`stat` check as 8.5) and its undo history
  starts over.
- **Closing:** Esc (when nothing is being typed or searched), Ctrl+W,
  Ctrl+Q (Command on macOS), the Close button, or the window's close box.

It is a separate process (the tray starts `pawprint settings` detached,
like the badge, and a thread reaps it), a normal window in the badge's own
backend in a third mode next to the badge and the preview: the shell runs
`settings::App` instead of `Badge`, and backends report the window's
pointer, wheel, keys, and typed text as `ui::Input` (`Shell::input`). It
serves no IPC and has no tray.

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
| `pawprint settings` | Open the settings window ([8.7](#87-the-settings-window)): every config key, for this host, all hosts, or any `[hosts]` table, saved as you click. It returns when the window closes. |
| `pawprint preview <id> [--size N]` | Open a temporary, normal (focusable, closable) window showing that mascot cycling through all its actions (every action in file order, `when` ignored, 1 s apart), on a slate backdrop with a 16 px margin, centered, at size 128 by default. Esc or closing the window ends it. It uses this host's label and accent and the badge's own backend surface in preview mode; it writes nothing. |
| `pawprint render <id> --out FILE.png [--size N] [--label TEXT] [--frame N]` | Write a badge image. Used for docs and golden tests. |
| `pawprint set <key> <value>` | Write `<key>` into this host's `[hosts."<short>"]` table using `toml_edit` (keeps comments and formatting), then reload ([8.6](#86-editing-the-config-set-unset-and-the-tray)). Example: `pawprint set mascot otter`. Exit code 2 for an unknown key or invalid value, 1 if the file can't be read or written. |
| `pawprint unset <key>` | Remove that key from this host's table, then reload. |
| `pawprint forget` | Drop this host's cached fallback assignment (through the running badge if there is one, which picks again and saves; else in the state file) and say what the hash picks now. A host whose mascot comes from another rule has nothing to forget. |
| `pawprint config init` | Write a commented `config.toml` if none exists. |
| `pawprint config path` | Print the config path. |
| `pawprint mascot export <id> <dir>` | Write a bundled mascot into the folder `<dir>` (created if needed; existing files are never overwritten) in the simple PNG form ([12.7](#127-png-bridges)) as a starting point for custom art. |
| `pawprint mascot check <dir>` | Validate a custom mascot directory with the same rules as the build ([12.11](#1211-validation-rules)), report alias collisions with the bundled catalog, and say whether it replaces a bundled mascot or adds one. Exits 1 on errors. |
| `pawprint autostart on\|off\|status` | Register or remove launch at graphical login for this user: an entry that runs `<this executable> run` (on Windows `pawprintw.exe run`). Linux: `$XDG_CONFIG_HOME/autostart/pawprint.desktop`. Windows and macOS: [11.1.5](#1115-tray-and-autostart), [11.2](#112-macos). |
| `pawprint doctor` | Diagnose the environment: session type, chosen backend and why, compositor/WM, layer-shell, ARGB visual, compositing manager, displays with names, scales and work areas, primary detection, GNOME fractional-scaling blur (with the fix), tray availability, IPC endpoint, autostart state, config warnings. |
| `pawprint paths` | Print every file location ([8.1](#81-files-and-locations)). |
| `pawprint gnome-extension install\|remove` | Plan B only ([11.5.3](#1153-plan-b-the-presenter-extension)): install or remove the GNOME presenter extension. Not built unless a GNOME check fails, so the command is hidden from `--help` and says it is not implemented. |
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
   `Forget`, `Settings`),
   and `ConfigTouched(Config)` (the shell noticed a change and re-read the
   file);
3. executes `core::badge::Effect`s: `Present(Frame)` (already scaled),
   `Place(PhysicalRect)`, `SetInteractive(bool)`, `SetFullscreen(policy)`
   (6.5), `SetTray(TrayState)`, `SaveState(State)` (write it atomically),
   `ReloadConfig` (re-read the user mascots into a new catalog for
   `Badge::set_catalog`, re-read the file, then send `ConfigTouched`),
   `OpenSettings` (start `pawprint settings` detached, 8.7), and `Exit`;
4. blocks until the next native event or `core.next_deadline()`.

Driving the core, in full (every backend's loop has this shape):

```rust
let inputs = Inputs { hostname, config, state, flags, seed: clock_seed };
let mut badge = Badge::new(Arc::new(Catalog::load(user_dir)), catalog::bundle(), inputs, Instant::now());
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
| `Shell::is_window()`, `Shell::title()` | When creating the surface: true for `pawprint preview` and `pawprint settings` (a normal window, with that title) |
| `Shell::input(&mut surface, input)` | For each pointer, wheel, key, or typed-text event of that normal window (`ui::Input`, positions in physical pixels from the content's top-left). The preview closes on Esc, Ctrl+W, or Ctrl+Q; the settings window takes it all |
| `Shell::target()` | Wayland, before committing: the core's target display, which fixes the output its layer surfaces go on |

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
| Unix calls | `libc`: `localtime_r` (the local hour, `Now::local_hour`), `setsid`, `close_range`/`fcntl` (detached start), `getuid`. Windows uses `GetLocalTime` from `windows-sys`. |

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

Without `XDG_RUNTIME_DIR`, Linux uses `/tmp/pawprint-<uid>/` (always
`/tmp`, never `$TMPDIR`, so every terminal in the session agrees).

On Linux the key comes from the environment alone, `WAYLAND_DISPLAY` first,
so a terminal in the session finds its badge without probing any display
server. GNOME on Wayland keys on `WAYLAND_DISPLAY` even though the badge
draws through XWayland. The X11 key keeps host and display number and drops
the screen (`:0`, `:0.0`, and `unix:0` are all `x11-0`; `localhost:10.0` is
`x11-localhost_10`). An absolute `WAYLAND_DISPLAY` has its `/` replaced.

Startup (`instance::claim`): bind the endpoint. If a live badge holds it,
print "already running" (with its pid if it answers `status`) and exit 0,
**whether or not it answers**: a stopped or stuck badge still owns its
session, so it is never replaced. Binding first means two badges started at
once can't both win.

On Unix the lock is an exclusive `flock(LOCK_EX | LOCK_NB)` on
`<key>.lock` next to the socket, taken before binding and held for the life
of the process. The kernel releases it when the process dies, however it
dies. Holding the lock, `bind` removes any socket left at the path (a
crashed badge's) and binds; failing to take it is `AddrInUse`. Only the
lock holder ever unlinks the socket, on a clean exit; the lock file stays.
Sockets are created mode `0600` in a `0700` directory that must be a real
directory (not a symlink) belonging to the user.

The session key and the transport are the only platform parts:
`instance::session_key()` per OS, and `ipc::Transport` (`bind`, `accept`,
`connect`, `remove_stale`), with `ipc::unix::UnixSocket` for Linux and
macOS and `ipc::windows::NamedPipe` for Windows. `bind` must fail with
`AddrInUse` while another live process holds the endpoint; that is what
makes it a lock.

The Windows key is `<user>-<session id>` (`GetUserNameW`, with `\` replaced
by `_`). The first pipe instance is created with
`FILE_FLAG_FIRST_PIPE_INSTANCE`, which fails with "access denied" when the
name exists; that maps to `AddrInUse`. The server connects one instance at a
time and creates the next before handing a connected one to the IPC thread,
so the name exists for as long as the badge runs and vanishes when it dies
(`remove_stale` has nothing to do). Every instance gets an explicit
security descriptor, `O:<user>D:P(A;;GA;;;<user>)(A;;GA;;;SY)` (the user SID
from `GetTokenInformation(TokenUser)`, built with `InitializeAcl`,
`AddAccessAllowedAce`, and `SetSecurityDescriptor*`, not parsed from SDDL,
see FW12): owned by the user,
and a protected DACL that lets only the user and SYSTEM open the pipe or add
an instance to it. No other account, anonymous included, can even open it
for reading. `PIPE_REJECT_REMOTE_CLIENTS` refuses network clients.

The client opens the pipe with `SECURITY_SQOS_PRESENT |
SECURITY_IDENTIFICATION`, so the server can't act as the client, and checks
that the pipe's owner (`GetSecurityInfo`) is its own user SID before it
sends anything. Another account that created the name first fails that
check (or refuses the open), which `claim` and the CLI report as an error
("the pipe belongs to another user") rather than "already running". The
owner, not the server process's token, is what is checked: the badge sets
it explicitly, so an elevated badge still matches, and no other non-admin
account can make itself our pipe's owner.

All pipe I/O is overlapped (`FILE_FLAG_OVERLAPPED`) and waits with
`GetOverlappedResultEx` for the 5-second IPC timeout, then `CancelIoEx`, on
both sides: a client that connects and never writes can't stall the IPC
thread, and a badge that never answers can't hang the CLI. The server's
`flush` waits (with the same timeout) for the client to read the answer and
close its end, so closing the server end never drops the answer.

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
malformed lines get an error response. The client gives up after 5
seconds; the badge's IPC thread waits at most 4 seconds for the loop and
then answers `{"ok":false,"error":"the badge did not answer in time"}`, so
the client hears why. An `ok:false` answer means a live badge (busy or
stuck), never a stale endpoint. A request line is at most 4 KiB. The CLI
and the badge always
come from the same binary, so the protocol carries no compatibility shims
beyond the version check.

### 10.6 Code sharing (DRY)

Platform code is unavoidable. Duplicated platform code is not. The rules:

1. **Backends translate, they never decide.** A backend turns native events
   into `core::Event`s and executes `core::Effect`s. Any `if` about layout,
   snapping, timing, labels, mascots, or config belongs in `pawprint-core`.
2. **Logic that appears in two backends moves to the core** in the change
   that introduces the second copy.
3. **Each backend stays small,** around 500–800 lines including its
   autostart glue. A tray in a module of its own (`backend::windows_tray`,
   `backend::menubar`, `backend::sni`) doesn't count toward its backend. A
   backend growing past that is a sign that rule 1 is being broken. Two are
   over, and were reviewed for it (M5): Wayland (about 1,050 lines) and X11
   (about 880). What they share is already in `backend::linux` (the loop,
   scale helpers, move-mode keys, doctor) and `backend::sni`; the excess is
   protocol plumbing with no decisions in it. Wayland carries sctk's handler
   traits (most methods empty), four kinds of surface (badge, work-area
   probe, move-mode catcher, preview window), and the keymap reader that
   replaces libxkbcommon (F22). X11 carries the EWMH, Motif, and shape
   setup, the image upload split to the request size, and the
   compositor-change rebuild.
4. **The core has a `FakeSurface`,** so move mode, display changes, reloads,
   and animation are tested in `make test` on every OS, without a window.

The backend surface is deliberately narrow:

```rust
pub trait Surface {
    fn displays(&self) -> Vec<DisplayInfo>;        // geometry, usable area, scale, primary, name
    fn present(&mut self, frame: &Frame);          // premultiplied BGRA, already scaled
    fn place(&mut self, rect: PhysicalRect);
    fn set_interactive(&mut self, on: bool);       // move mode on/off
    fn set_fullscreen(&mut self, policy: Fullscreen); // hide under fullscreen windows, or not (6.5)
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
| Layout, clamping, 3 × 3 snap, label fitting | `pawprint-core::layout` | All backends |
| Move mode: drag math, constraint, nudge, Esc, 30 s timeout | `pawprint-core::movemode` | All backends report raw pointer/key events only |
| Animation scheduler, `next_deadline()` | `pawprint-core::anim` | All backends |
| Compositor, integer scaler, font | `pawprint-core::render` | All backends, `render`, `preview`, golden tests |
| **One pixel format**: premultiplied little-endian ARGB8888 (bytes B, G, R, A) plus a 1-bit mask | `pawprint-core::render` | Windows `UpdateLayeredWindow`, X11 32-bit visual, `wl_shm` `ARGB8888`, macOS `CGImage`, presenter extension (F30); the mask feeds the X11 shape fallback |
| Settings window: pages, controls, scopes, undo, hit-testing, drawing (8.7) | `pawprint-core::settings` on `pawprint-core::ui` (window input, a clipped canvas in units, theme, integer upscale) | All backends, through `Shell` in its settings mode; `make settings-shots` |
| Fullscreen coverage rule (6.5) | `pawprint-core::surface::fullscreen_covers` | Windows, X11 |
| Keysyms to window input (text, editing keys, Ctrl shortcuts), XKB keysym names | `pawprint::backend::linux` (`window_input`, `keysym_from_name`) | X11, Wayland |
| Tray menu model (items, checks, actions) | `pawprint-core::menu` | Windows tray (`backend::windows_tray`), macOS menu bar (`backend::menubar`), `ksni` |
| Tray icon scaling (`render::tray_icon`: nearest neighbor, binary alpha, optional margin) | `pawprint-core::render` | Windows tray, macOS menu bar |
| Sprite compiler | `pawprint-sprite` crate | `build.rs`, runtime user mascots, `pawdev`, `pawprint mascot check` |
| IPC protocol and server loop | `pawprint::ipc` | Unix-socket transport shared by Linux and macOS; named-pipe transport on Windows |
| Autostart interface (`on`/`off`/`status`) | `pawprint::autostart` | Three small implementations behind one interface: Linux XDG entry, Windows HKCU `Run`, macOS LaunchAgent |
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
9. Out of the way of fullscreen windows on its display, unless
   `fullscreen = "show"` ([6.5](#65-fullscreen-windows)).

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
- The loop (`backend::windows::run`, and `preview` without IPC): drain the
  message queue, hand the events and tray choices `wndproc` recorded and the
  queued IPC requests to the shell, `tick`, then
  `MsgWaitForMultipleObjectsEx(QS_ALLINPUT, MWMO_INPUTAVAILABLE)` with
  `Shell::timeout()` (input that arrived before the last peek still wakes
  it). The
  IPC thread queues each request on a channel and wakes the loop with
  `PostMessageW(WM_APP)`. `wndproc` only records core events and tray
  choices in thread-local queues: Win32 calls such as `SetWindowPos` send
  messages synchronously, and this keeps the shell from being re-entered.
  `WM_CLOSE` and `WM_QUIT` quit cleanly.
- **Normal window** (`pawprint preview` and `pawprint settings`,
  `Shell::is_window`): the same window and loop, made normal. Class
  `PawprintPreview`, title `Shell::title()` (`PAWPRINT preview` or
  `PAWPRINT settings`),
  style `WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX` with no
  extended style, so it has a taskbar button, takes the focus, and is not
  topmost or click-through. It isn't layered: `Present` copies the frame
  into the DIB section and `WM_PAINT` `BitBlt`s it; `Place` grows the
  client rectangle by the frame (`AdjustWindowRectExForDpi`) and shows the
  window activated. Windows keeps a captioned window at least `SM_CXMIN`
  wide (120 px of client at 96 DPI on the runner), so a narrower preview is
  widened about its center and the frame is centered on the class's
  backdrop brush. Its input goes to `Shell::input`: `WM_MOUSEMOVE`,
  `WM_LBUTTONDOWN`/`UP` (with capture), and `WM_MOUSEWHEEL` (3 lines a
  notch) in client coordinates less the centering offset; `WM_KEYDOWN`
  for editing keys and Ctrl+letter shortcuts; `WM_CHAR` (the loop calls
  `TranslateMessage`) for typed text. `WM_CLOSE` quits (the settings window
  first finishes a field being typed in).
- Fullscreen windows (6.5): with `fullscreen = "hide"` the badge window is
  an appbar for `ABN_FULLSCREENAPP` (callback `WM_APP + 1`) and a
  foreground WinEvent hook is set; either sets a flag that the loop turns
  into one check of the foreground window. The tray keeps working while the
  window is hidden.
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
  background process only with permission: every IPC client calls
  `AllowSetForegroundWindow(<badge pid>)` as it connects, with the pid from
  `GetNamedPipeServerProcessId` (`pawprint move` runs in the foreground
  terminal; for other commands it does nothing), and the badge calls
  `SetForegroundWindow` on entering move mode and hands the foreground back
  to the previous window when it ends. Move from the tray menu finds the
  badge in the foreground already (the menu took it), so the previous window
  is the one that was in the foreground when the menu opened; the badge
  itself never counts as one. Where
  Windows refuses, dragging still works. A click never activates the badge
  (`WS_EX_NOACTIVATE`, `MA_NOACTIVATE`).

#### 11.1.5 Tray and autostart

- Tray: `backend::windows_tray::NotifyIcon` implements `shell::Tray`,
  attached with `Shell::set_tray` once the badge is shown; the badge's
  `wndproc` passes it the tray messages. One `Shell_NotifyIconW` icon
  (id 1) on the badge window, `NOTIFYICON_VERSION_4`, with the callback
  message `WM_APP + 1` and the tooltip from `TrayState`. The icon is the
  rest frame in its crate (`TrayState.icon`, 32 × 32) at the small-icon size
  for the primary display's DPI (`GetSystemMetricsForDpi(SM_CXSMICON)`: 16
  px at 100%, 32 at 200%), scaled by the core's `render::tray_icon`
  (nearest neighbor, so it keeps binary alpha), and redrawn when that size
  changes (`WM_DPICHANGED`, `WM_DISPLAYCHANGE`, `WM_SETTINGCHANGE`).
  `tray = false` removes it (`NIM_DELETE`); quitting removes it before the
  window goes.
- Menu: **Move**, **Size ▸ 32/64/128**, **Animation ▸ Idle/Static**,
  **Settings…**, **Reset position**, **Reload config**, **Quit**, built on
  each opening
  from the core's menu model: an item with children is a submenu, a
  `checked` item gets a check mark, and each command gets an id.
  `NIN_SELECT` (left click), `NIN_KEYSELECT` (keyboard), and
  `WM_CONTEXTMENU` (right click) all open it at the point the shell gives,
  with the classic rule: `SetForegroundWindow` on the owner first, then
  `TrackPopupMenu(TPM_RETURNCMD)`, then a posted `WM_NULL`. `wndproc`
  queues the chosen command and the loop hands it to `Shell::menu`, so
  Size and Animation are saved like `pawprint set` (8.6). The menu's modal
  loop runs inside `wndproc`, so animation and IPC wait while it is open.
- `TaskbarCreated` (registered when the tray is attached) adds the icon
  again after Explorer restarts, and `wndproc` keeps what it needs (the icon
  data and the menu model) in a thread-local, so neither needs the shell.
  Both that message and the callback pass `ChangeWindowMessageFilterEx`, so
  an elevated badge still hears from the unelevated Explorer.
- Autostart (`pawprint::autostart`):
  `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value
  `pawprint` = `"<dir>\pawprintw.exe" run` (`REG_SZ`), where `<dir>` holds
  the running `pawprint.exe`. `status` reads the value back, `off`
  deletes it (already off is fine). HKCU Run entries run at every
  interactive logon, RDP logons included. Settings → Apps → Startup and
  Task Manager can switch the entry off without removing it, in the binary
  value `HKCU\…\Explorer\StartupApproved\Run\pawprint` (first byte odd,
  `0x03`, when off): `status` then adds "disabled in Startup apps", and
  `on` and `off` delete that value, so `on` means on.

#### 11.1.6 Two executables on Windows

A GUI-subsystem executable has no attached console, so CLI output is lost
and the shell does not wait for it. A console-subsystem executable flashes a
console window when started at logon. PAWPRINT therefore ships **both** from
the same code (two `[[bin]]` targets with one shared `main`):

- `pawprint.exe` (console subsystem): the CLI. Bare `pawprint` starts
  `pawprintw.exe run` detached.
- `pawprintw.exe` (`#![windows_subsystem = "windows"]`): the badge, and the
  autostart target.

The code is the `pawprint` library crate (`src/lib.rs`, `pawprint::main`);
`src/main.rs` and `src/bin/pawprintw.rs` are three-line wrappers. On other
systems `pawprintw` builds as a stub that points at `pawprint`. Bare
`pawprint` starts the `pawprintw.exe` next to it, falling back to itself,
with `CreateProcessW` and `bInheritHandles = FALSE`, so the badge holds
none of the CLI's handles (FW8), and `DETACHED_PROCESS |
CREATE_NEW_PROCESS_GROUP` (no console, no Ctrl+C from the terminal). It
first asks for `CREATE_BREAKAWAY_FROM_JOB`, so a terminal that runs its
children in a job doesn't take the badge along when it closes, and retries
without it where the job forbids breakaway. The process handle tells
whether the badge exited early (`WaitForSingleObject(0)`,
`GetExitCodeProcess`). On Unix the detached badge gets `setsid`, null
stdio, and every descriptor from 3 up marked close-on-exec
(`close_range(CLOSE_RANGE_CLOEXEC)`, else `fcntl` up to `_SC_OPEN_MAX`), for
the same reason. A GUI-subsystem process still writes to stdout and stderr
handles it inherits, so `pawprintw run --verbose` with redirected output logs to that
file.

#### 11.1.7 Known limits

- Exclusive-fullscreen games and the secure desktop (UAC, lock screen) cover
  the badge. This is expected, and with `fullscreen = "hide"` (6.5) the
  badge hides under any fullscreen foreground window anyway.
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
  `collectionBehavior = canJoinAllSpaces | stationary | ignoresCycle`, plus
  `fullScreenAuxiliary` only with `fullscreen = "show"` (6.5; without it
  the panel stays out of fullscreen apps' Spaces, F39),
  `hidesOnDeactivate = false`. It is ordered in with
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
- Autorelease pools: without `NSApplicationMain` nothing drains
  autoreleased objects, so setup runs inside an `autoreleasepool` and every
  turn of the loop inside one of its own. The GUI-session test runs the
  badge with `OBJC_DEBUG_MISSING_POOLS=YES` and fails on any "MISSING
  POOLS" line for the main thread (whose `pthread_self` the badge logs).
  AppKit's own helper threads leak a few objects at startup and when the
  status item is created (FM10).
- Event loop: `nextEventMatchingMask(any, untilDate: deadline,
  NSDefaultRunLoopMode)` waits for the next event or `Shell::timeout()`.
  The IPC thread, menu choices, and the screen-change observer wake it by posting an
  application-defined `NSEvent` from the main dispatch queue. Every event
  the badge doesn't consume goes to `NSApplication.sendEvent`, so the status
  item's menu runs from there (while a menu is open,
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
- Menu bar (`backend::menubar`, the `shell::Tray`): a square
  `NSStatusItem`, created after `finishLaunching` and attached with
  `Shell::set_tray` after `Shell::start`. Its button image is the 32 × 32
  rest frame in an 18-point box with a 1-point margin (`render::tray_icon`), as two bitmaps
  (36 px at 2x, the art 1:1; 18 px at 1x, every other art pixel), so AppKit
  never interpolates; it is colorful, so not a template image. The tooltip
  is the tray tooltip. The `NSMenu` is rebuilt from `TrayState.menu` on
  every change: an item with children is a submenu, `checked` is the
  item's state checkmark, and each command item's tag indexes a command
  list held by one target object (`PawprintMenuTarget`). Its `choose:`
  action only queues `Inbox::Menu(command)` on the channel IPC uses and
  wakes the loop, which calls `Shell::menu`. `tray = false` removes the
  item; turning it back on creates a new one.
- Normal window (`pawprint preview` and `pawprint settings`): the same
  loop without IPC or menu-bar item. The app is **Regular** (a Dock icon
  and a place in the app switcher, so the window can be found again) and
  the window an ordinary `NSWindow`, titled `Shell::title()`, closable and
  miniaturizable, at the normal level, taking mouse (moves included,
  `acceptsMouseMovedEvents`) and keyboard. It is made key and the app
  activated at the first `Place` (`activateIgnoringOtherApps`, since
  `activate` needs macOS 14). The core's rect is the content rect; the
  frame adds the title bar. Its events go to `Shell::input`: mouse moves,
  drags, and clicks below the title bar at `locationInWindow` in pixels
  from the content's top-left (and still to AppKit, which activates the
  window; the title bar stays AppKit's), the wheel
  (`scrollingDeltaY`), editing keys by `kVK_*` code, Command+letter as a
  shortcut, and other keys' `characters` as text (function keys' private
  use characters dropped), all swallowed so nothing beeps.
  `NSWindowWillCloseNotification` sends `Command::Quit`.
- Autostart: `~/Library/LaunchAgents/io.github.nealhardesty.pawprint.plist`
  with `ProgramArguments = [<path>/pawprint, run]`, `RunAtLoad = true`,
  `LimitLoadToSessionType = Aqua` (GUI sessions only),
  `KeepAlive = { SuccessfulExit = false }` (restart on crash, not on
  `pawprint quit`; `pawprint run` exits 0 when a badge already runs, so a
  login with a running badge doesn't loop), and `ProcessType = Interactive`.
  `autostart on` writes it and, if its content changed while the agent is
  loaded, unloads it (`launchctl bootout`; a failure is logged), since
  launchd reads the plist only on load. Unless the agent is still loaded, it loads
  it now with `launchctl bootstrap gui/<uid> <plist>`, which starts the
  badge (a failure, e.g. from an SSH login, is logged; the agent still
  loads at the next GUI login). `off` runs `launchctl bootout
  gui/<uid>/io.github.nealhardesty.pawprint`, which stops a badge launchd
  started, and removes the file. `status` reads the file's
  `ProgramArguments` and says whether launchd has the agent loaded
  (`launchctl print`). macOS 13+ shows a "Background Items Added"
  notification and lists the agent in System Settings → General → Login
  Items.
- Gatekeeper: binaries fetched with `curl` don't get the
  `com.apple.quarantine` attribute, so curl-installed binaries run without a
  prompt. Browser downloads are quarantined. The README documents
  `xattr -d com.apple.quarantine`.
- Apple Silicon requires every arm64 executable to carry a signature. The
  Rust toolchain ad-hoc signs at link time. After `lipo` creates the
  universal binary, `make package` re-signs it ad hoc with
  `codesign --force -s -` (`make release` does it, then checks it; confirmed, F15).

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
- Fullscreen windows (6.5): with `fullscreen = "hide"` the backend selects
  `PropertyChange` and `StructureNotify` on the active window and unmaps
  the badge while that window is fullscreen over the badge's display; it
  maps it again (with the same hints) when it isn't. No WM, no
  `_NET_ACTIVE_WINDOW`, nothing to do.
- Normal window (`pawprint preview`, `pawprint settings`): managed, with
  `WM_DELETE_WINDOW`, and `PointerMotion | LeaveWindow` selected. Motion,
  button 1, buttons 4 and 5 (the wheel, 3 lines), and leave go to
  `Shell::input` in window coordinates; keys through the shared
  `linux::window_input` from the keycode's two keysyms
  (`GetKeyboardMapping`), with Shift, Lock, and Control from the event's
  state.

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
- Surface: layer `top` (`overlay` with `fullscreen = "show"`, changed in
  place with `set_layer`, 6.5), namespace `pawprint`, `exclusive_zone = 0`,
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
  position inside the output isn't known. Anchor, margins, and size are
  set before the surface's first commit: a zero size on an axis not
  anchored to both edges is a protocol error (KWin enforces it; sway
  doesn't). `make test-wayland` checks the order in the `WAYLAND_DEBUG`
  log.
- Primary output: Wayland has no core concept of a primary output. On
  Plasma, `kde_output_order_v1` lists outputs in priority order, and the
  first is the primary. Elsewhere: the configured `display` name, a single
  output, or else the first output by position (not the null output, which
  would leave the core without a usable area to lay out in).
- The output of a layer surface is fixed at creation (F5). It is the core's
  target display (`Shell::target`, [6.2](#62-target-display)); the backend
  never picks one. The badge is recreated when the target changes, when
  its output goes away, and when the compositor closes it.
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
  keymap it falls back to the evdev codes. Shift is XKB modifier 0, Lock
  1, Control 2. The same reader records each key's first two keysyms
  (`linux::keysym_from_name`) for the normal window's typing.
- Normal window (`pawprint preview`, `pawprint settings`): an xdg
  toplevel titled `Shell::title()`, fixed size (so tiling compositors
  float it). Its pointer events (enter and motion, button `BTN_LEFT`,
  leave, the vertical axis) go to `Shell::input` at surface coordinates
  times the window's scale, and keys through `linux::window_input`. There
  is no key repeat: a held key types once.
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
| **Crate** | `assets/crate/crate.toml` | Crate back, crate front, and label strip, drawn with accent-ramp glyphs so any accent tints them |
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
prop_offset = [0, 0]                # optional: moves template props, see 12.4 (default [0, 0])

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
  Every op offset, prop position, and `prop_offset` component is within
  −64 to 64 (twice the frame); anything larger is an error.
- **`follows = "<slot>"`** on a layer makes its slot take that slot's
  offsets as well as its own, so `head+1,0` carries the eyes, ears, nose, and
  mouth that follow `head`. Chains are allowed (`eyes` follows `disc`
  follows `head`); loops and unknown slots are errors. Every layer of a slot
  that sets `follows` must name the same slot. When a mascot replaces a base
  layer and doesn't set `follows`, it keeps the base layer's. `follows`
  affects offsets only, not visibility.
- Props are drawn after the mascot (and after `flipx`) at mascot
  coordinates; `all+` does not move them. **`prop_offset = [dx, dy]`** adds
  to the position of every prop a *motion template* draws, and never to the
  `prop:` ops in the mascot's own actions. A base may set it; a mascot's own
  value replaces the base's. PNG mascots can't use templates, so they can't
  set it. Props are clipped to the frame, so keep the moved props inside it.
- `ms` defaults to 200 and `weight` to 5 (1–1000). A template action takes the
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
right (`zzz` rising at x 21, `bang` at x 26, `question` at x 24, `heart` at
x 22), so keep that corner for them where the silhouette allows. When it
can't (a side-on animal facing right, a wide cap, a spout), set
`prop_offset` to move every template prop to the mascot's emptiest corner:
whale, dolphin, pufferfish, gecko, swan, mushroom, cactus, and teapot do.

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
  opossum, cat-black, and ghost) are livelier at night: their `when = "night"` actions
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
     with 700 ms of rest between. The sheet is sampled on a 100 ms clock
     and each GIF frame stores only the pixels that changed, which keeps
     all 100 mascots in about 1.3 MB.
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
  error. `pawprint mascot export <id> <dir>` produces this simpler form:
  the strip holds every distinct compiled frame, rest first. Because a PNG
  action has one `duration_ms`, export picks the duration (at least 80 ms)
  that best fits the action's frame times and lists a held frame several
  times; times with a common divisor of 80 ms or more come out exact, and
  the rest are within 50 ms per frame (checked for every bundled mascot).

**User mascots** live in `<config dir>/mascots/<id>/` ([8.1](#81-files-and-locations)),
in either form: text (with `extends`, motion templates, and props from the
embedded library, [12.13](#1213-embedding)) or PNG. They are compiled with
the user rules of [12.11](#1211-validation-rules) once per process and
shared by the badge and every command that uses the catalog (`run`,
`explain`, `list`, `render`); `pawprint mascot check` uses the same rules.

- The folder name must equal the `id`. A folder whose id matches a bundled
  mascot **replaces** it (`list` shows `override`); a new id **adds** a
  mascot (`user`), eligible for the fallback unless it sets
  `fallback = false`.
- User aliases win collisions with bundled ones (rule 8). Each collision is
  a warning naming both mascots.
- An invalid user mascot is skipped with a warning (in the log for the
  badge, on stderr for commands); the bundled mascot of the same id, if
  any, stays. User mascots never stop the badge from starting.
- The badge reads the folder when it starts and on every reload (`pawprint
  reload`, `set`/`unset`, the tray, or a changed `config.toml`): the shell
  loads a fresh `Catalog` and hands it to the badge (`Badge::set_catalog`,
  which owns its catalog as an `Arc`), then re-reads the config. A mascot
  whose folder was removed is resolved away at once.

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
  `eyes.happy`. On light fur (white, cream, pale yellow) a white highlight
  merges with the fur and the eye reads as a dash at 1×, so those mascots
  map the `h` role to `grey-3`: a solid dot at 1×, a soft catchlight when
  large. On black fur, use a light iris or tan brow points so the eye shows.
- **Whiskers and bristles that stick out past the silhouette** have no
  outline, so they use a mid tone (`grey-2`, as on the walrus and seal) that
  reads on both light and dark desktops. White ones vanish on a light
  desktop.
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
   are satisfied. Op offsets, prop positions, and `prop_offset` are within
   −64 to 64.
4. Compiled frames: every alpha is 0 or 255 (always true for grids, checked
   for PNG mascots). More than 16 colors is a warning, and a warning in a
   bundled mascot fails the build.
5. At least one action. Weights 1–1000. Durations 80–2000 ms. Each action
   ≤ 4000 ms total.
   Every bundled mascot has a signature (non-template) action.
6. The rest frame has opaque pixels in rows 0–19.
7. `accent` resolves.
8. Squashed aliases are unique across the catalog (bundled check), also with
   their digits dropped. For user mascots, collisions are a warning, and the
   user mascot wins. An id or alias that contains a digit never matches a
   host name ([5.3](#53-choosing-a-mascot)), so it is an error in a bundled
   mascot and a warning in a user mascot.
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
catalog. The result, a `pawprint_sprite::Bundle` (accents, both fonts, crate,
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
│   ├── crate/crate.toml      # crate back/front, label strip (text source)
│   ├── font/font.toml        # 5x7 bitmap font (text source)
│   ├── font/micro.toml       # 3x5 micro font for long labels (text source)
│   ├── bases/                # shared body plans (12.8)
│   ├── motions/              # motion templates (12.4)
│   ├── props/                # shared props (12.2)
│   └── mascots/<id>/mascot.toml
├── installers/
│   ├── install.sh
│   └── install.ps1
├── tests/
│   └── golden/               # expected badge renders (PNG) and hostnames.tsv (catalog stability)
├── extras/
│   └── gnome-presenter/      # Plan B GNOME Shell extension (only if needed, 11.5.3)
├── docs/
│   ├── ART.md                # mascot style guide with worked examples
│   ├── gallery.png           # generated contact sheet (make contact-sheet; gallery.gif with ANIMATED=1)
│   ├── building-blocks.png   # every base x every template, props, accents (make building-blocks)
│   └── settings.png          # the settings window's mascot page (make settings-shots)
├── spikes/
│   └── f22-static-musl/      # standalone spike crate (not a workspace member); make spike-f22
├── README.md  SPEC.md  ROADMAP.md  AGENTS.md  LICENSE
├── .github/workflows/ci.yml       # calls make targets only
└── .github/workflows/release.yml  # on a v* tag: package, publish, finalize (make targets)
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
- The Makefile does no OS detection: every recipe is the same on every
  OS, and whatever differs per OS (release targets, binaries, shells,
  paths) is decided in Rust by pawdev at `cfg!` time.
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
| `settings` | Open the settings window from the local build (`pawprint settings`, [8.7](#87-the-settings-window)) |
| `power-check` | Measure the release badge against [7.2](#72-power-budget): Linux under Xvfb, macOS on the current desktop with a private HOME (from a universal binary), Windows on the current desktop (the user's config and state files are backed up on disk as `<file>.pawdev-backup` and put back afterwards, or at the next run after an interruption); `POWER_SECONDS=120` per mode |
| `test` | Unit tests, golden render tests, and asset validation |
| `check-assets` | Validate the mascot catalog only |
| `lint` | `cargo fmt --check` and `clippy -D warnings` |
| `fmt` | Format the code |
| `golden` | Regenerate golden images after an intentional rendering change |
| `golden-hosts` | Recompute the catalog stability fixture (`tests/golden/hostnames.tsv`) after an intentional catalog change |
| `contact-sheet` | Render every mascot, labels hidden, grouped by base, into `docs/gallery.png` (`ANIMATED=1` adds `docs/gallery.gif`) |
| `building-blocks` | Render the crate accents, props, and every base with every motion template into `docs/building-blocks.png` |
| `settings-shots` | Render every page of the settings window into `out/settings/<page>.png`, and the mascot page into `docs/settings.png` for the README |
| `palette` | Regenerate `assets/palette.gpl` from `assets/palette.toml` |
| `art-export` | Export a mascot's layers and frames as PNGs: `make art-export MASCOT=owl` |
| `art-import` | Convert a PNG layer into a grid: `make art-import MASCOT=owl PNG=eyes.png` |
| `ci` | `lint` + `test` (what CI runs on every OS) |
| `release` | Optimized build for this platform, checked and run ([14.5](#145-release-builds)): static musl x86_64 + aarch64 on Linux, universal re-signed ad hoc on macOS, MSVC on Windows |
| `package` | `release` + archives + `.sha256` sidecars into `dist/` ([15.2](#152-release-assets)), plus `dist/COMMIT`; enforces catalog gates |
| `publish` | Upload `dist/*` and installers to the GitHub release `v<version>`, creating it as a draft if needed (needs `gh`, a clean tree, the tag on `HEAD`, and `dist/` packaged from that commit); `DRY_RUN=1` prints the `gh` commands only |
| `release-finalize` | After every OS has run `publish`: check the draft has all four archives, their sidecars, and both installers, then publish it ([15.3](#153-release-procedure)); `DRY_RUN=1` prints the `gh` commands and the expected assets only |
| `install` | `package`, then install for this user through the real installer with `--archive dist/<archive>` (autostart on, badge started) |
| `uninstall` | Remove the install and autostart through the installer (config is kept) |
| `test-install` | `package`, then run the installers end to end ([15.7](#157-installer-tests)); a CI job runs it on all three OSes. On macOS and Windows it refuses to run while this user's autostart is on, unless `CI` is set |
| `lint-installers` | `shellcheck --shell=sh` on `install.sh` |
| `doctor` | Run `pawprint doctor` from the local build |
| `clean` | Remove `target/` and `dist/` |
| `check-all-targets` | `cargo check` of the whole workspace for every release target, from any OS (needs no linkers) |
| `spike-f22` | Rebuild the F22 spike ([14.4](#144-static-linux-binaries)) for x86_64 and aarch64 musl |
| `test-x11` | Linux: the X11 backend under Xvfb, bare, with `xcompmgr`, and with `openbox` ([16.1.1](#1611-display-server-tests-linux)) |
| `test-wayland` | Linux: the Wayland backend under headless sway ([16.1.1](#1611-display-server-tests-linux)) |
| `test-linux-display` | Both of the above (what the Linux display CI job runs) |
| `test-macos-display` | macOS: the real badge in the logged-in desktop session ([16.1.2](#1612-gui-session-test-macos)); a `macos-latest` CI job runs it |
| `test-windows-display` | Windows: the real badge on this desktop ([16.1.3](#1613-desktop-test-windows)); a `windows-latest` CI job runs it |

Every target is implemented; a pawdev test checks that this table and the
Makefile list the same targets. `make package` runs the catalog gates
first.

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

### 14.5 Release builds

`make release` (logic in `pawdev release`) builds `pawprint` with
`cargo build --release` for this OS's targets and checks every binary
before anything is packaged:

| OS | Builds | Checks |
|---|---|---|
| Linux | `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl` | No `PT_INTERP` program header (static or static-pie); runs `--version` natively, and the aarch64 one under `qemu-aarch64-static` when installed (else it says it skipped) |
| macOS | `aarch64-apple-darwin` + `x86_64-apple-darwin` → `lipo -create` → `target/universal-apple-darwin/release/pawprint` → `codesign --force -s -` | `codesign -v`, `lipo -archs` lists `x86_64 arm64`, runs `--version`; reports whether the x86_64 slice runs (Rosetta) |
| Windows | `x86_64-pc-windows-msvc`: `pawprint.exe` and `pawprintw.exe` | Runs both with `--version` |

Every binary must be under the 15 MB budget ([7.2](#72-power-budget)) and
print `pawprint <workspace version> (`.

Release profile: `lto = true`, `codegen-units = 1`, `strip = true`,
`opt-level = 3` (the default), unwinding panics. Measured in M5 with all
100 mascots (Linux musl, stripped):

| Profile | x86_64 | aarch64 | Archive (x86_64 `.tar.gz`) |
|---|---|---|---|
| **lto + codegen-units 1 + strip (chosen)** | **7.74 MB** | **6.83 MB** | 2.73 MB |
| + `panic = "abort"` | 6.84 MB | 5.97 MB | 2.30 MB |
| + `panic = "abort"` + `opt-level = "s"` | 5.57 MB | — | — |

The other release builds (M5, from CI): macOS universal 8.02 MB (both
slices; 2.29 MB archive), Windows `pawprint.exe` and `pawprintw.exe`
4.5 MB each (2.37 MB zip). All are well under the 15 MB budget.

Neither `panic = "abort"` nor `opt-level = "s"` is used. The binary is
already about half the budget, so size buys nothing. `opt-level = 3` keeps rendering cheapest, which is what the
power budget cares about. And with `abort`, a CLI panic becomes a `SIGABRT`
with a core dump (and on Ubuntu an apport crash report) instead of exit 101:
today `pawprint explain | head` panics on the broken pipe, so abort is not
safe until the CLI handles `EPIPE`.

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

`make package` (logic in `pawdev package`) writes them into `dist/`. The
archives are flat (the files at the top level, no directory), with
`pawprint` at mode 0755 and the documents at 0644, owned by uid 0. The zip is
deflated. A sidecar is one line, `<lowercase hex>  <archive name>`, so
`sha256sum -c` and `shasum -a 256 -c` accept it. pawdev writes the archives
itself (with the `tar`, `flate2`, and `zip` crates), so packaging needs no
`tar` or `zip` tool and works the same way on every OS.

### 15.3 Release procedure

1. Bump the version in `Cargo.toml`, update the README if needed, and commit
   to `master`.
2. Tag `v<ver>` on that commit and push the tag.
3. The tag push runs `.github/workflows/release.yml`, which uses the same
   make targets a person would:
   - **Linux** checks out the tag and runs `make package && make publish`.
     That creates the release **as a draft** and uploads both musl archives,
     their sidecars, and the installers.
   - **macOS** and **Windows** run the same two targets once Linux is done.
     They wait because draft releases don't reserve their tag, so two jobs
     creating the release at once would make two drafts.
   - **`make release-finalize`** then lists the draft's assets, refuses
     while any of the ten is missing (four archives, four sidecars, two
     installers) and names them, and publishes the draft. Nobody sees a
     draft, so `latest` never points at a release that is missing an OS.
4. Run the installer one-liners on a clean machine of each OS.

The workflow is a convenience, not a dependency. The same steps work by
hand: check out the tag on each OS, run `make package && make publish`
(Linux first), then `make release-finalize` from any machine. Re-running
`make publish` replaces same-named assets.

`make publish` (logic in `pawdev publish`) refuses to run, and says how to
fix it, when:

- `git status --porcelain` shows anything. Untracked files count, because
  the build embeds everything under `assets/`.
- the tag `v<ver>` doesn't exist, or doesn't point at `HEAD`.
- `dist/COMMIT` is missing or names another commit. `make package` writes
  the `HEAD` it built from there (with `-dirty` appended if the tree had
  changes), so archives left over from another commit, or built from a
  dirty tree, are never uploaded under the tag.
- `dist/` holds an archive of another version.

Otherwise it runs these from the repository root:

```sh
gh release view v<ver> --repo nealhardesty/pawprint        # does it exist?
gh release create v<ver> --repo nealhardesty/pawprint --draft --title 'PAWPRINT <ver>' --generate-notes --verify-tag   # only if not
gh release upload v<ver> --repo nealhardesty/pawprint --clobber dist/<every archive and sidecar> installers/install.sh installers/install.ps1
```

`make release-finalize` (logic in `pawdev finalize`) runs:

```sh
gh release view v<ver> --repo nealhardesty/pawprint --json assets --jq '.assets[].name'
gh release edit v<ver> --repo nealhardesty/pawprint --draft=false      # only if nothing is missing
```

With `--verify-tag`, `gh` refuses if the tag was not pushed.
`make publish DRY_RUN=1` and `make release-finalize DRY_RUN=1` print their
commands (and anything a real publish would refuse, or the expected asset
list) without running them. CI runs both on every OS.

Archives are reproducible: every file in them carries the commit time of
`HEAD` (or `SOURCE_DATE_EPOCH` when set) as its modification time, and the
gzip header has none, so packaging the same binaries twice gives the same
bytes and the same sidecar.

The repository is public, so standard GitHub-hosted runners are free. CI runs `make ci` on all three OSes for every push and PR. Moving
step 3 into CI is a roadmap item; local builds remain fully supported.

### 15.4 `install.sh` (Linux, macOS)

```sh
curl -fsSL https://github.com/nealhardesty/pawprint/releases/latest/download/install.sh | sh
curl -fsSL .../install.sh | sh -s -- --version 1.2.0 --no-autostart
```

Steps:

1. Parse the flags. Detect the OS (`uname -s`) and the CPU (`uname -m`:
   x86_64 or aarch64 on Linux; macOS always gets the universal build).
2. Resolve the version. For `latest` (the default), follow GitHub's
   redirect from `releases/latest` to `releases/tag/v<ver>` and read the
   version from the tag (`curl -w '%{url_effective}'`, or the `Location:`
   headers of `wget --spider -S`). Asset names carry the version, so
   `releases/latest/download/<asset>` works for the installers but not for
   the archives.
3. Download `releases/download/v<ver>/<archive>` and its `.sha256` into a
   temporary directory (removed on exit), with `curl`, or `wget` if there is
   no curl. With `--archive FILE`, use that file instead, plus `FILE.sha256`
   if it sits next to it. Without a sidecar, the installer says the
   checksum is not checked.
4. Verify with `sha256sum` or `shasum -a 256`. A mismatch stops here.
5. Unpack, and run the new binary's `--version`. A binary that can't run on
   this machine stops here. On macOS, clear `com.apple.quarantine` from it
   (an archive downloaded with a browser is quarantined; curl's isn't).
6. **Only now change the system.** Stop a running badge (`pawprint quit`,
   then wait until `pawprint status` says it's gone). Copy the binary next
   to `~/.local/bin/pawprint` and `mv` it into place, which is atomic.
7. `pawprint autostart on`. If this fails, the installer warns and carries
   on: the binary is installed.
8. Start the badge with bare `pawprint`. This fails without a graphical
   session (over SSH, say); the installer warns that the badge will start at
   the next graphical login.
9. Print `pawprint explain`, and a PATH hint if the prefix is not on `PATH`.

Flags: `--version V`, `--archive PATH` (install from a local archive, with no
network access), `--prefix DIR`, `--no-autostart`, `--no-start`,
`--uninstall`, `--purge` (with uninstall, also delete config and state),
`--help`. Environment equivalents: `PAWPRINT_VERSION`, `PAWPRINT_PREFIX`.
`PAWPRINT_DOWNLOAD_BASE` (default
`https://github.com/nealhardesty/pawprint/releases`) points the installer at
a mirror with the same layout. The installer tests use it to reach a local
server. The repository is public, so downloads need no credentials.

The script is POSIX `sh` with no bashisms. It is checked with
`shellcheck --shell=sh` and runs under dash, bash, and macOS's bash 3.2 as
`sh`. It uses only `curl` or `wget`, `tar`, and a SHA-256 tool, and changes
nothing until verification passes.

### 15.5 `install.ps1` (Windows)

```powershell
irm https://github.com/nealhardesty/pawprint/releases/latest/download/install.ps1 | iex
& ([scriptblock]::Create((irm https://github.com/nealhardesty/pawprint/releases/latest/download/install.ps1))) -Version 1.2.0 -NoAutostart
```

Same steps and flags as `install.sh` (`-Version`, `-Archive`, `-Prefix`,
`-NoAutostart`, `-NoStart`, `-Uninstall`, `-Purge`, `-Help`, and the same
environment variables). Installs to `%LOCALAPPDATA%\Programs\pawprint\`,
adds that directory to the **user** `PATH`, runs `Unblock-File` on the
binaries, and verifies with `Get-FileHash -Algorithm SHA256`. It works on
Windows PowerShell 5.1 and PowerShell 7. Piping into `iex` is not subject to
the script execution policy.

Details that matter:

- `latest` comes from the redirect's `ResponseUri` (a `HEAD`
  `WebRequest`). Downloads use `Invoke-WebRequest -UseBasicParsing` with
  TLS 1.2 enabled, because 5.1 may default to older protocols. The progress
  bar is off, because it slows 5.1 downloads badly.
- The script never calls `exit`: run through `iex`, that would close the
  user's window. Errors are thrown instead, and `-File` turns them into
  exit code 1.
- Native commands run with `$ErrorActionPreference = 'Continue'`, because
  under `Stop`, 5.1 turns redirected stderr into terminating errors.
- A running or just-exited exe can be renamed but not overwritten. So each
  binary is moved to `<name>.old` before the new one is copied in, and the
  `.old` file is deleted when possible (otherwise on the next run).
- The user `PATH` is edited in the registry (`HKCU\Environment`) as
  `REG_EXPAND_SZ`, which keeps entries like `%USERPROFILE%\bin` unexpanded.
  A `WM_SETTINGCHANGE` broadcast (sent by
  `[Environment]::SetEnvironmentVariable`) tells Explorer. The current
  session's `$env:Path` is updated too.
- The badge is started with `Start-Process` (ShellExecute), which passes
  no handles down: started directly, the long-lived badge would inherit the
  shell's stdout and hold open the pipe of anything capturing the
  installer's output (CI, fleet tools) until it quits.
- The script is ASCII only: Windows PowerShell 5.1 reads a script without
  a byte-order mark in the ANSI code page.

### 15.6 Upgrades and uninstall

Re-running the installer upgrades in place: it stops the running badge
(`pawprint quit`), replaces the binary, and starts it again. Config, state,
and user mascots are in separate directories and are never touched except by
`--purge`. A failed download, checksum, or smoke run changes nothing, and
the running badge keeps running.

Uninstall runs `pawprint autostart off` and `pawprint quit`, then removes
the binaries. If an autostart entry is still there afterwards (from an older
binary, or with no binary at all), it removes the entry directly: the XDG
desktop file, the LaunchAgent plist, or the `HKCU\…\Run` value `pawprint`.
On Windows it also removes the directory from the user `PATH`, and deletes
the directory if it is empty. `--purge` deletes the config and state
directories ([8.1](#81-files-and-locations)), plus `~/Library/Logs/pawprint`
on macOS. To uninstall a `--prefix` install, pass the same `--prefix`.

`make install` runs the same installer with `--archive dist/<this machine's
archive>` after `make package`, and `make uninstall` runs it with
`--uninstall`. A local install takes exactly the path a download does.

### 15.7 Installer tests

`make test-install` (logic in `pawdev test-install`) runs after
`make package`. It drives the real installers under every interpreter on
the machine: `sh`, `dash`, and `bash` on Linux; `sh` (bash 3.2) and `bash`
on macOS; `powershell` (5.1) and `pwsh` on Windows. Each run gets a
throwaway home. On Unix that means `HOME`, plus a private `XDG_RUNTIME_DIR`
and `TMPDIR` so that a badge already on the desktop is left alone; on
Windows, `LOCALAPPDATA` and `APPDATA`. Each run also gets a local HTTP
server laid out like GitHub releases: a `latest` redirect, a good release,
and a release whose `.sha256` is wrong.

1. Fresh install from `--archive`. The installed binary is the packaged one
   and prints the version. The autostart entry runs it. The output has the
   `explain` text and the PATH hint (on Windows, the directory was added to
   the user `PATH`).
2. Upgrade from `latest` over HTTP. The installer says it is upgrading.
   Where there is a desktop session (the macOS and Windows runners, a Linux
   desktop), the badge was running before and is running again after.
3. `--version` from the bad release. It fails and says "checksum". The
   installed binary's bytes and mtime are unchanged, and the running badge
   still runs.
4. `--uninstall`. The binaries and the autostart entry are gone, no badge
   runs, and the config is kept (on Windows, the directory left the user
   `PATH`).
5. Install with `PAWPRINT_PREFIX` + `PAWPRINT_VERSION` and `--no-start
   --no-autostart` (no autostart entry), then `--uninstall --purge`. The
   config and state are gone.

6. Install from a relative `--archive` (on Unix in a directory with a
   backslash in its name, which GNU `sha256sum` would escape) into a
   relative `--prefix` (on Windows with a trailing `\`), run from the
   throwaway home. The checksum is checked, and the installer reports the
   absolute install path.
7. Uninstall it through `--prefix '~/rel bin'` (Unix; the installer
   expands a `~` the calling shell left alone) or the relative prefix
   (Windows).

On Linux, step 5's purge also sets relative `XDG_CONFIG_HOME` and
`XDG_STATE_HOME`, which must be ignored as the XDG spec says.

Autostart: on Linux the test checks the XDG entry; on Windows and macOS,
that `autostart status` reports the installed binary (and `off` after
uninstall). On Windows and macOS the badge and the autostart entry are the
user's real ones, so the test refuses to start while `pawprint autostart
status` reports on, unless the `CI` environment variable is set (CI runners
are clean machines).

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
  synthetic names under the bundled catalog, with the rule that chose each,
  in `tests/golden/hostnames.tsv` (checked by `pawdev`'s tests). A change
  here means assignments moved on fresh installs, which requires a conscious
  update: `make golden-hosts` recomputes the fixture (append a hostname on
  its own line to add one), and the diff goes in the PR.
- **Alias audit:** `pawprint-core`'s `tests/catalog.rs` fails if any alias
  is a common hostname word or if any of a corpus of ordinary hostnames
  (`web-01`, `db-prod-3`, `lab3`, `monitor-01`, ...) matches a mascot by rule
  3 or 4 ([Appendix A](#appendix-a-launch-catalog)). It also checks that
  every nocturnal mascot has a night action and that every bundled mascot
  survives `pawprint mascot export` and recompiles to the same frames.
- **Layout:** anchor/offset ↔ rectangle round-trips, clamping, the 3 × 3
  snap, and label fitting: the candidate each label gets at P = 1, 2, 3,
  4, cutting from the right, the `label_size` caps, and the badge staying
  exactly 32P × 32P for every label.
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
- **The settings window** (8.7) driven by clicks, keys, and typing on its
  own hit targets: every key has a control, scopes write the right table,
  inheritance and shadowing are shown, number fields refuse bad values and
  keep intervals ordered, host tables are added, renamed, and deleted,
  undo and failed writes, a broken file is never written, search, hover
  previews, and window sizing per display.
- **Shell pieces:** backend selection (§3.2), scale helpers, the IPC
  protocol and a real socket round trip, session keys, log rotation, and the
  Wayland keymap reader.

#### 16.1.1 Display-server tests (Linux)

`make test-x11` and `make test-wayland` run the real `pawprint run` against
real display servers and check the screen. They are `#[ignore]`d in
`make test` because they need the servers installed; a separate Linux CI
job installs them (`xvfb xcompmgr openbox sway swaybg grim foot wtype dbus`) and
runs `make test-linux-display`. Each test gets its own HOME and runtime
directory and fails with the badge's log. Xvfb picks a free display itself
(`-displayfd`) and admits only clients with the test's cookie (`-auth`,
never `-ac`). Child processes get SIGTERM, then SIGKILL after a second, so
Xvfb removes its lock file.

| Suite | Session | What is asserted |
|---|---|---|
| X11 | Xvfb, no WM, no compositor | override-redirect, depth 24 (shaped); DOCK type and ABOVE/STICKY/SKIP_* state; screen pixels equal `pawprint render` on opaque pixels and the backdrop on transparent ones; an XTest click on the label reaches the window below; in move mode a drag moves the badge, saves the snapped anchor and offset, and is pixel-exact at the new spot; arrow keys, Shift, Enter, and Escape; a `_NET_WORKAREA` change and an `Xft.dpi` change re-lay it out |
| X11 | Xvfb + `xcompmgr` started and stopped under a running badge | the window is rebuilt as 32-bit ARGB and back; pixel-exact and click-through in both |
| X11 | Xvfb + `openbox` | a managed DOCK, at the requested position, above a normal window mapped later, clicks pass through to that window (F32), drag works |
| X11 | GNOME-like environment (`XDG_CURRENT_DESKTOP=GNOME`, no layer-shell) | the `gnome` profile is chosen, ARGB, keyed on `WAYLAND_DISPLAY` |
| X11 | Xvfb | single instance: a second `run` and bare `pawprint` say "already running", even while the first is stopped (SIGSTOP) and not answering; after a crash the stale socket is replaced; bare `pawprint` detaches; `quit` removes the socket |
| Wayland | headless sway, pixman renderer, `swaybg` backdrop | layer surface pixel-exact at top-right (via `grim`); a virtual-pointer click on the badge focuses the `foot` window under it; in move mode the click is taken |
| Wayland | headless sway | move-mode drag far outside the badge (F7) saves the right position and is pixel-exact; `wtype` arrow keys, Enter, Escape with exclusive keyboard |
| Wayland | headless sway + `swaybar` | the badge sits below the bar's exclusive zone (F3), pixel-exact |
| Wayland | headless sway | output mode change; scale 2 crisp (every opaque pixel exact); scale 1.5 sized 96 px and placed right (crispness at fractional scales is an owner check, F34) |
| X11 | Xvfb, move mode | the dashed accent outline appears on the badge edge (a dash in the transparent corner, art through a gap) and goes when move mode ends |
| X11 | Xvfb + private `dbus-daemon` + a stand-in StatusNotifierWatcher (the test's own zbus object) | the item registers with `Id`, `Title`, `ToolTip`, and a 32 and 64 px `IconPixmap`; the dbusmenu layout has the shared menu with the right checks; **Size ▸ 128** (a dbusmenu `Event`) resizes the badge and appends `[hosts.owl] size = 128` after the untouched `config init` template; **Animation ▸ Static** is saved; `set`/`unset` reload the running badge; an unknown mascot is refused (exit 2) without writing; a user mascot added while the badge runs shows after `set mascot`; a hand edit is picked up at the next IPC request (mtime); `tray = false` removes the item and `unset tray` brings a new one; **Quit** exits; the saved choice survives the restart |
| X11 | Xvfb | `pawprint preview owl`: a normal window (not override-redirect, no DOCK type, `WM_DELETE_WINDOW`), centered, the badge pixel-exact on the backdrop, the frame changes as actions play; Esc closes it, and so does a `WM_DELETE_WINDOW` message |
| Wayland | headless sway | `pawprint preview owl`: an xdg toplevel (`app_id` `pawprint`) that sway floats (fixed size), focused, the badge pixel-exact on the backdrop; Esc closes it |
| X11 | Xvfb + `openbox` | 6.5: a managed window made fullscreen and active through EWMH client messages unmaps the badge, leaving fullscreen maps it again, `fullscreen = "show"` keeps it mapped |
| X11 | Xvfb | `pawprint settings`: a normal window, centered, 720 × 504; an XTest click on the grid writes this host's mascot, typed `moose` and Return pick it, Esc closes |
| Wayland | headless sway + `foot` | 6.5: `fullscreen enable` covers the badge (top layer), `disable` uncovers it, `fullscreen = "show"` keeps it above |
| Wayland | headless sway | `pawprint settings`: a floating window 720 px wide; a virtual-pointer click on the grid and `wtype` typing write config.toml; Esc closes |

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
Enter, and Escape, and chooses **Size ▸ 32** from the menu-bar item (a
posted click opens it, posted arrow keys walk it, Return chooses), which
must shrink the badge and write `size = 32` into `[hosts.owl]`. The
menu-bar item must go with `tray = false` and come back: it is found as
the menu-bar window (layer 25 along the top of the screen, whoever owns
it; Control Center on macOS 26, FM6) that turning `tray` off removes. `autostart on` must write the LaunchAgent (checked with
`plutil -lint` and for every key), load it (`launchctl print
gui/<uid>/io.github.nealhardesty.pawprint` succeeds and shows the badge it
started), and `off` must boot it out, stop that badge, and remove the file
(skipped if the agent is already loaded for the user). `pawprint preview
owl` must open a Regular app's window titled "PAWPRINT preview" at layer 0,
160 points wide (128 + 2 × 16), without a menu-bar item, in front of other
normal windows (by window-server order), that Escape and then its close
button (pressed through Accessibility, FM9) each end with exit 0.
`pawprint settings` must open a Regular app's window at layer 0, the
content 360 × 252 units at 2 / scale points each, that writes nothing until
used; with Accessibility permission a posted click on the mascot grid
writes this host's mascot, posted typing and Return pick a searched one,
and Escape (twice: the search first) ends it with exit 0. In
static mode the badge's loop must not wake for 15 s (one debug line per
wakeup; FM7). Without a permission, that check prints `SKIP` and why. Click-through, Spaces, full-screen apps, and Retina scales stay owner
checks ([16.2](#162-backend-smoke-checks-manual-per-backend-every-release)).

#### 16.1.3 Desktop test (Windows)

`make test-windows-display` runs `pawprintw.exe run --verbose` (stderr to a
file) on the current desktop and checks it with Win32 calls from the test
(`crates/pawprint/tests/windows.rs`). The test's own windows run on a thread
that keeps dispatching messages, so they never stall the shell's
synchronous broadcasts, and a watchdog fails the test after two minutes. It is `#[ignore]`d in `make test`; a
`windows-latest` CI job runs it. It refuses to start while a badge runs in
the session, and restores the user's state file, config file, and autostart value afterwards.
Win32 known folders can't be redirected, so it uses the real profile.

| Check | How |
|---|---|
| Styles | `GetWindowLongPtrW`: `WS_POPUP`; `LAYERED`, `TRANSPARENT`, `TOPMOST`, `TOOLWINDOW`, `NOACTIVATE`; no `APPWINDOW`; never the foreground window |
| Placement | top-right of the primary work area, 16 logical px in; `GetWindowRect` equals `status` |
| Topmost and pixels | a normal solid window shown *after* the badge covers the work area; a `BitBlt` with `CAPTUREBLT` of the badge equals `pawprint render` on opaque pixels and the backdrop on transparent ones (at scale 1) |
| Click-through | `WindowFromPoint` skips the badge, and a `SendInput` click on the label reaches the window below |
| Move mode | `pawprint move` clears `TRANSPARENT`; `WindowFromPoint` finds the badge on the art and the backdrop on a transparent pixel; a `SendInput` drag moves it, saves the position, and is pixel-exact; arrow keys and Enter when Windows grants the foreground |
| Work area | an app bar docked at the top (40 px) moves the badge down 40 px, and back when it goes (FW5) |
| IPC and instances | `status`, `move`, `reset-position`, `quit` over the pipe; a second `run` says "already running"; bare `pawprint` starts `pawprintw.exe` detached. The pipe itself is covered by `ipc::windows`'s unit tests in `make test`: its owner and DACL are exactly the user and SYSTEM, a client that connects and never writes doesn't stall the server, an unserved pipe times the client out after 5 s, and a pipe owned by Administrators (where the token may assign that owner) is refused |
| Tray | `Shell_NotifyIconGetRect` finds the badge's icon (else a no-op `NIM_MODIFY` must succeed); a posted `WM_CONTEXTMENU` callback opens the menu, which is read from the test process (`MN_GETHMENU`: the six labels, Size ▸ 64 checked); `SendInput` clicks on **Size** and **128** resize the badge in place and write `size = 128` into `[hosts.owl]`; `pawprint unset size` brings it back; after a click on the backdrop window, **Move** from the menu enters move mode, and when Esc ends it the foreground goes back to the backdrop window, not the badge |
| Autostart | `autostart on` writes the `Run` value `"<dir>\pawprintw.exe" run` (read back with `RegGetValueW`), `status` shows it, `off` deletes it, a second `off` says "already off"; a `StartupApproved\Run` value starting with `0x03` makes `status` say "disabled in Startup apps" and `on` deletes it; the user's values are restored |
| Preview | `pawprintw.exe preview owl --size 64` opens `PawprintPreview`: caption and system menu, no `WS_POPUP`, none of the overlay's extended styles, title `PAWPRINT preview`, a client area 96 px tall and at least 96 wide on the slate backdrop; `WM_CLOSE` and Esc each end it with exit code 0 |
| Fullscreen (6.5) | A borderless window over the whole primary monitor, brought to the front, hides the badge window (not destroyed); closing it shows the badge again; with `fullscreen = "show"` it stays visible |
| Settings (8.7) | `pawprintw.exe settings` opens a `PawprintPreview` window titled `PAWPRINT settings` with a client area of 360 × 252 times the scale; a posted click on the mascot grid writes this host's mascot, posted `WM_CHAR`s and Return pick a searched one, and Esc ends it with exit code 0 |

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
8. A fullscreen window on the badge's display (a full-screen remote client
   among them) hides it, and leaving fullscreen brings it back (6.5).
9. Tray → **Settings…** opens the settings window; a change there shows on
   the badge at once, typing works in its fields, and Undo takes it back
   (8.7).

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
| F15 | Apple Silicon requires signed arm64 code; ad-hoc signing suffices for local execution. | Re-sign after `lipo` | **Confirmed** on `macos-latest` (Apple Silicon, `make test-install`): `codesign` reports "replacing existing signature" on the `lipo` output, then `codesign -v` passes, `lipo -archs` is `x86_64 arm64`, the binary runs (`aarch64-apple-darwin` slice) and so does the x86_64 slice under Rosetta; the curl-installed copy ran and started the badge with no prompt |
| F16 | `LimitLoadToSessionType = Aqua` restricts a LaunchAgent to GUI sessions. | No badge attempts in SSH sessions | Documented; an Aqua agent bootstrapped into `gui/<uid>` from the GUI session loads and starts the badge (`make test-macos-display`). The SSH side is unverified |
| F17 | X11 ARGB transparency needs a compositing manager (`_NET_WM_CM_S<n>` owner). Without one, the Shape extension's bounding mask gives 1-bit transparency. | Binary alpha + shape fallback | **Confirmed** (Xvfb with and without `xcompmgr`, switched live; both pixel-identical to `pawprint render`) |
| F18 | xrdp and Xvnc sessions are X11 even when the console runs Wayland. | X11 backend covers Linux RDP/VNC sessions | Documented |
| F19 | macOS ships GNU Make 3.81. | Makefile stays 3.81-compatible | Documented |
| F20 | GNU Make on Windows uses `sh.exe` if found on `PATH`, else `cmd.exe`. | Single-command recipes | Documented |
| F21 | Standard GitHub-hosted runners are free for public repositories. | CI on all three OSes | Documented |
| F22 | `x11rb`, pure-Rust `wayland-client`, `smithay-client-toolkit`, `calloop`, `zbus`, and `ksni` link into a fully static musl binary for x86_64 and aarch64 with only rustup pieces (`linker = "rust-lld"`, bundled musl CRT). x86_64: static-pie, 4.9 MB stripped, connects to X11 (XFixes 5.0, RandR), Wayland, and the session bus; exits cleanly with no session. aarch64: statically linked, 4.3 MB (not run; no qemu). Needs sctk without `xkbcommon` and wayland-client without `system`/`dlopen`/`log`. | One Linux binary for every distro; no cross toolchain for aarch64 | **Confirmed (M0 spike, `make spike-f22`)**; **re-confirmed on the real badge in M5** (`make release`): x86_64 `static-pie linked`, aarch64 `statically linked` (`file`), `ldd`: "statically linked", no `PT_INTERP`; both run `--version`, aarch64 under `qemu-aarch64-static` (qemu-user-static 8.2, Ubuntu 24.04) |
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
| F33 | `x11rb` 0.14 connects through the `/tmp/.X11-unix` socket or TCP, not the abstract socket namespace. Where `/tmp/.X11-unix` is read-only (WSLg), a test Xvfb listens on abstract sockets only. | Tests (and `power-check`) run Xvfb on the filesystem socket where `/tmp/.X11-unix` is writable, else with `-listen tcp`; either way with `-auth` and a private cookie, never `-ac`. Real X servers always create the filesystem socket | **Confirmed** |
| F34 | Headless sway 1.9 with the pixman renderer filters the whole output at fractional scales (even swaybg's solid color comes out uneven); about 85% of the badge's pixels at 1.5 were exact. At integer scales every pixel is exact. | Fractional-scale crispness can't be judged there | **Confirmed on headless sway**; crispness at 125%/150% is an owner check on Plasma |
| F35 | wlroots `keyboard_interactivity = exclusive` on an overlay layer surface sends it the keys at once; virtual keyboards (`wtype`, remote-desktop servers) send keycodes from their own keymaps. | Exclusive keyboard in move mode; keycodes read from the keymap (11.4) | **Confirmed on sway 1.9**; verify on Plasma |
| F36 | zbus 5 (async-io) runs its socket setup (peer credentials, the SASL zero byte) on the `blocking` crate's thread pool. From `blocking` 1.7 an idle pool thread never exits (it exits only above the thread limit) and wakes every 500 ms; 1.6 lets it exit after 500 ms idle. | `blocking` is pinned to `=1.6.2`, so the badge has zero wakeups at rest with the tray running (7.2) | **Confirmed** (`make power-check`: 39 wakeups in 20 s in static mode with 1.7.0, 0 with 1.6.2) |
| F37 | `toml_edit` keeps a document's trailing comments after its last table, so a table added to a file that ends in comments lands above them. | The `config init` template ends with an empty `[hosts]`, so host tables go after the commented examples (8.6) | **Confirmed** (`config_edit` tests) |
| F38 | A `ksni` item with `MENU_ON_ACTIVATE` and the dbusmenu layout can be driven entirely over D-Bus: a stand-in StatusNotifierWatcher on a private `dbus-daemon` receives the registration, and a dbusmenu `Event(id, "clicked")` runs the item's callback. | The tray is tested end to end in `make test-x11` without a desktop (16.1.1) | **Confirmed**; real hosts (Plasma, Ubuntu's AppIndicator) are owner checks |
| F39 | A `canJoinAllSpaces` panel at `NSStatusWindowLevel` without `fullScreenAuxiliary` is not shown in another app's fullscreen Space (Jump Desktop, Screens, a fullscreen browser), and is with it. | macOS hides the badge under fullscreen apps with no detection (6.5) | **Verify** (owner, real Mac with a fullscreen remote client) |
| F40 | Mutter puts DOCK-type windows in its bottom layer while their monitor shows a fullscreen window, native Wayland or XWayland. | GNOME steps aside for native Wayland fullscreen windows, which the X11 backend can't see; with `fullscreen = "show"` Mutter may still lower the badge (6.5) | **Verify** (owner, Ubuntu) |
| F41 | The shell sends `ABN_FULLSCREENAPP` to an appbar registered with `ABM_NEW` alone (no `ABM_SETPOS`, so no space reserved) when `mstsc` (and other borderless windows covering a monitor) goes full screen or leaves it, including from windowed mode without a foreground change. | Windows needs no polling to catch a window going fullscreen in place (6.5) | **Verify** (CI covers the foreground case; mstsc in place is an owner check) |
| F42 | KWin draws an active fullscreen window above the layer-shell `top` layer and below `overlay`, like sway. | Plasma hides the badge under fullscreen windows by layer alone (6.5) | **Confirmed on sway** (`make test-wayland`); **verify** on Plasma |

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
| FM6 | On macOS 26 an `NSStatusItem`'s window belongs to **Control Center**, not the app: `CGWindowListCopyWindowInfo` lists it as an `Item-0` window at layer 25 along the top of the screen, owned by Control Center's pid. A posted click on it opens the app's `NSMenu`, posted arrow keys walk the menu and its submenus, and Return chooses. | The test finds the item by turning `tray` off and on, and drives the menu with posted input (16.1.2) | **Confirmed** on the runner (macOS 26.6) |
| FM7 | In static mode the badge's main thread sleeps in `nextEventMatchingMask` with no event for as long as measured, but the process still shows a few interrupt wakeups a second and about 0.03% CPU from AppKit's and the system's own threads, with or without the menu-bar item. | The static-mode budget holds for the badge's loop; the process-wide numbers are reported, not failed (7.2) | **Confirmed** on the runner |
| FM8 | `NSEvent.subtype` raises an Objective-C exception for event types without a subtype (mouse and key events), which aborts a Rust process. | Never read `subtype` on arbitrary events | **Confirmed** (a diagnostic log line did it) |
| FM9 | A posted click in a titled window's title bar that misses the traffic lights drags the window. The close button's inset differs between releases (macOS 26 has a 32-point title bar). | The test presses the preview's close button through Accessibility (`AXCloseButton` via System Events), not by coordinates | **Confirmed** on the runner |
| FM10 | An `NSApplication` run without `NSApplicationMain` has no autorelease pool on its main thread: with `OBJC_DEBUG_MISSING_POOLS=YES` every turn of the loop logged leaked events, dates, images, and menu objects (thousands per minute of the GUI-session test). AppKit's own secondary threads also leak a few hundred small objects (sets, arrays, strings, `NSXPCInterface`) at launch and when a status item is created, outside any code the badge runs. | Pools around setup and each loop turn (11.2); the test checks the main thread only | **Confirmed** on the runner (macOS 26) |
| FM11 | On a Retina Mac running macOS 26, `CGWindowListCopyWindowInfo` reports a titled window's bounds a few points inside its AppKit frame (a 720 × 536 frame as 706 × 526; a 144-point-wide preview as 142). | GUI tests locate a normal window's content from its middle and aim at the middle of controls, never at exact edges (16.1.2); the preview test's exact-width check holds only on the runner's scale-1 display | **Confirmed** (a developer's Mac) |

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
| FW6 | `AllowSetForegroundWindow` (then `ASFW_ANY`, now the badge's pid) from the process running `pawprint move`, then `SetForegroundWindow` in the badge, gives the badge the keyboard when that process may set the foreground. | Arrow-key nudging works from a terminal (11.1.4) | **Confirmed on the runner**; verify from Windows Terminal and conhost on a desktop |
| FW7 | `CreateNamedPipeW` with `FILE_FLAG_FIRST_PIPE_INSTANCE` fails with `ERROR_ACCESS_DENIED` when the name exists, and a pipe name vanishes when its last server handle closes. | The pipe is the instance lock; no stale endpoints (10.4) | **Confirmed** (`make test` on windows-latest; the desktop test's crash-free restarts) |
| FW8 | `std::process::Command` on Windows creates children with handle inheritance on, so a child inherits every inheritable handle, including the parent's own redirected stdout and stderr, even when its stdio is set to null. | Bare `pawprint` starts the badge with `CreateProcessW(bInheritHandles = FALSE)` (11.1.6); otherwise `pawprint \| more` or a script capturing its output would wait until the badge quits. The same holds for inherited descriptors on Unix, which the detached start marks close-on-exec | **Confirmed** (the desktop test hung until this was fixed; the Unix side by `instance`'s unit test) |
| FW9 | `Shell_NotifyIconGetRect` returns a newly added notification icon's rectangle on the runner (`[797, 720, 32, 48]`, in the taskbar below the 720-px work area): new icons are shown, not hidden in the overflow. | The desktop test finds the tray icon directly; a no-op `NIM_MODIFY` is its fallback where an icon sits in the overflow | **Confirmed** |
| FW10 | Another process can read and drive a menu shown by `TrackPopupMenu`: `FindWindowExW("#32768")` + `MN_GETHMENU` give its `HMENU`, `GetMenuStringW`, `GetMenuState`, `GetSubMenu`, and `GetMenuItemRect(NULL, ..)` work on it, and `SendInput` clicks choose items (`TPM_RETURNCMD` returns the id). | The tray menu is tested end to end with real clicks (16.1.3) | **Confirmed** |
| FW11 | Windows keeps a `WS_CAPTION \| WS_SYSMENU \| WS_MINIMIZEBOX` window at least `SM_CXMIN` wide: a 96-px client area came out 120 px at 96 DPI. | The preview widens about its center and centers the frame on its backdrop brush (11.1.1) | **Confirmed** |
| FW12 | `ConvertStringSecurityDescriptorToSecurityDescriptorW` leaves the process with extra threads: building the pipe's descriptor from SDDL took the idle badge from 2 threads to 4 and from 0 to 2 wakeups in 150 s of static mode. | The descriptor is built with the plain ACL calls (10.4) | **Confirmed** (`make power-check` on windows-latest) |

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
| `dog-greyhound` | greyhound | sky | Long nose rests on the crate edge, sigh |
| `dog-pug` | pug | honey | Head tilt, snort |
| `dog-dachshund` | dachshund, wiener, doxie | copper | Long body slides further out, then back |
| `dog-labrador` | dog, labrador | honey | Happy pant, tail wag |
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
| `crow` | raven, corvid | sky | Tilts its head, caws silently |
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
| `beetle` | — | pine | Shell opens a crack, wings peek |
| `snake` | python, viper | moss | Tongue flick, sway |
| `bee` | bumblebee, honeybee | marigold | Bobbing hover, buzz lines |
| `snail` | slug | ochre | Eye stalks extend and retract |

### Objects and oddballs (8) — category `oddballs`

| ID | Extra aliases | Accent | Signature action |
|---|---|---|---|
| `crt` | — | charcoal | Screen flickers, prompt cursor blinks |
| `floppy` | diskette | slate | Shutter slides open, closed |
| `mushroom` | shroom, fungus | cranberry | Cap bounces, spores puff |
| `ghost` | boo, spooky | ice | Floats up, fades a step |
| `ufo` | saucer | teal | Hovers, beam flickers |
| `robot` | droid | slate | Antenna light blinks, head swivels |
| `cactus` | — | pine | A flower pops open |
| `teapot` | tea, kettle | rose | Lid rattles, steam puff |

Totals: 32 + 24 + 16 + 12 + 8 + 8 = **100**.

Alias collisions are resolved at catalog time. For example, `cat` and `dog`
belong only to `cat-orange-tabby` and `dog-labrador`, and `cow` is not an
alias of `cat-cow` (it stays free for a future `cow`).

**Alias audit (M3).** No alias may be a word that machines are commonly
named with (`db`, `web`, `prod`, `lab`, `bug`, `monitor`, ...): rules 3 and
4 would send every `lab-3` or `monitor-01` to the same mascot, which defeats
telling machines apart. The M3 audit removed eight aliases: `bug` (beetle),
`lab` (dog-labrador), `grey` (dog-greyhound), `monitor`, `tv`, and
`terminal` (crt; terminal servers are common among remote-desktop users),
`disk` (floppy), and `bot` (robot). Hostnames that name the animal itself
still match (`beetle-1`, `labrador-01`, `crt`). `pawprint-core`'s
`tests/catalog.rs` enforces this with a list of common hostname words and a
corpus of ordinary hostnames that must match no alias. Mascot ids are not
audited: a host named `elk` or `fox` gets that animal on purpose.

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
