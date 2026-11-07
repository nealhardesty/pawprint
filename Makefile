# PAWPRINT: every task goes through make (SPEC §14). Run `make` to list targets.
#
# Rules: GNU Make 3.81 compatible (macOS ships 3.81). Every recipe line is a
# single command that runs cargo or pawdev, so it behaves the same under sh
# and cmd.exe. Real logic lives in tools/pawdev, never in shell.

CARGO ?= cargo
PAWDEV = $(CARGO) run --quiet --package pawdev --
ARGS ?=
MASCOT ?= owl
SIZE ?= 128
PNG ?=
ANIMATED ?=
POWER_SECONDS ?= 120

# Every release target (keep in sync with rust-toolchain.toml).
TARGETS = x86_64-unknown-linux-musl aarch64-unknown-linux-musl x86_64-apple-darwin aarch64-apple-darwin x86_64-pc-windows-msvc

.DEFAULT_GOAL := help
.PHONY: help build run preview power-check test test-x11 test-wayland test-linux-display test-macos-display test-windows-display check-assets lint fmt golden golden-hosts contact-sheet building-blocks palette art-export art-import ci release package publish install uninstall doctor clean check-all-targets spike-f22

help: ## List targets
	@$(PAWDEV) help $(firstword $(MAKEFILE_LIST))

build: ## Debug build of pawprint (and pawprintw on Windows)
	$(CARGO) build --workspace

run: ## Build and run the badge in the foreground; ARGS="..." passes arguments
	$(CARGO) run --package pawprint -- run $(ARGS)

preview: ## Open a window cycling through a mascot's actions: make preview MASCOT=owl SIZE=128
	$(CARGO) run --package pawprint -- preview $(MASCOT) --size $(SIZE)

power-check: ## Measure the release badge against the power budget (SPEC 7.2): Linux under Xvfb, Windows on this desktop; POWER_SECONDS=120 per mode
	$(PAWDEV) power-check $(POWER_SECONDS)

test: ## Unit tests, golden render tests, and asset validation
	$(CARGO) test --workspace
	$(PAWDEV) check-assets

test-x11: ## Linux: drive the X11 backend under Xvfb, with and without xcompmgr and openbox (needs those installed)
	$(CARGO) test --package pawprint --test x11 -- --ignored --test-threads=1

test-wayland: ## Linux: drive the Wayland backend under headless sway, checked with grim (needs sway, swaybg, grim, foot, wtype)
	$(CARGO) test --package pawprint --test wayland -- --ignored --test-threads=1

test-linux-display: test-x11 test-wayland ## Linux: both display-server test suites

test-macos-display: ## macOS: drive the badge in the logged-in desktop session and check its window, level, and pixels
	$(CARGO) test --package pawprint --test macos -- --ignored --test-threads=1 --nocapture

test-windows-display: ## Windows: run pawprintw.exe on this desktop and check its window styles, placement, click-through, move mode, IPC, and pixels
	$(CARGO) test --package pawprint --test windows -- --ignored --test-threads=1 --nocapture

check-assets: ## Validate the mascot catalog only (catalog gates are reported, not enforced)
	$(PAWDEV) check-assets

lint: ## cargo fmt --check and clippy -D warnings
	$(CARGO) fmt --all -- --check
	$(CARGO) clippy --workspace --all-targets -- -D warnings

fmt: ## Format the code
	$(CARGO) fmt --all

golden: ## Regenerate golden images after an intentional rendering change
	$(PAWDEV) golden

golden-hosts: ## Recompute the catalog stability fixture (hostname to mascot) after an intentional catalog change
	$(PAWDEV) golden-hosts

contact-sheet: ## Render every mascot, labels hidden, grouped by base, into docs/gallery.png (ANIMATED=1 adds docs/gallery.gif)
	$(PAWDEV) contact-sheet $(if $(ANIMATED),--animated)

building-blocks: ## Render the crate accents, props, and every base with every motion template into docs/building-blocks.png
	$(PAWDEV) building-blocks

palette: ## Regenerate assets/palette.gpl (for GIMP, Aseprite, LibreSprite) from assets/palette.toml
	$(PAWDEV) palette

art-export: ## Export a mascot's layers and frames as PNGs into out/art/<id>/: make art-export MASCOT=owl
	$(PAWDEV) art-export $(MASCOT)

art-import: ## Print a PNG layer as a grid in the mascot's glyphs: make art-import MASCOT=owl PNG=out/art/owl/eyes.png
	$(PAWDEV) art-import $(MASCOT) $(PNG)

ci: lint test ## lint + test (what CI runs on every OS)

release: ## Optimized build for this platform (universal on macOS, musl on Linux)
	@$(PAWDEV) todo release M5

package: ## release + archive + .sha256 into dist/; enforces catalog gates
	$(PAWDEV) check-assets --package
	@$(PAWDEV) todo package M5

publish: ## Upload dist/* and installers to the GitHub release for the current version (needs gh)
	@$(PAWDEV) todo publish M5

install: ## Install the local release build for this user and enable autostart
	@$(PAWDEV) todo install M5

uninstall: ## Remove the local install and autostart (config is kept)
	@$(PAWDEV) todo uninstall M5

doctor: ## Run pawprint doctor from the local build
	$(CARGO) run --quiet --package pawprint -- doctor

clean: ## Remove target/ and dist/
	$(PAWDEV) clean-dist
	$(CARGO) clean

check-all-targets: ## Type-check the workspace for every release target (no linkers needed)
	$(CARGO) check --workspace --all-targets $(addprefix --target ,$(TARGETS))

spike-f22: ## Spike F22: link x11rb, wayland-client, sctk, zbus, and ksni into static musl binaries
	$(CARGO) build --release --manifest-path spikes/f22-static-musl/Cargo.toml --target x86_64-unknown-linux-musl --target aarch64-unknown-linux-musl
