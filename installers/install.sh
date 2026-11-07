#!/bin/sh
# PAWPRINT installer for Linux and macOS (SPEC §15.4).
#
#   curl -fsSL https://github.com/nealhardesty/pawprint/releases/latest/download/install.sh | sh
#   curl -fsSL .../install.sh | sh -s -- --version 1.2.0 --no-autostart
#
# POSIX sh: runs under dash, busybox ash, and macOS's bash 3.2. It needs curl
# or wget, tar, and sha256sum or shasum, and changes nothing on the system
# until the download is verified and the new binary has run. Everything is
# in functions and runs from the last line, so a download cut short runs
# nothing.

set -eu

RELEASES=https://github.com/nealhardesty/pawprint/releases

usage() {
    cat <<'EOF'
Install PAWPRINT, the pixel-art hostname badge, for this user.

usage: install.sh [options]

  --version V     install version V (default: the latest release)
  --archive FILE  install from a downloaded archive, with no network access
                  (FILE.sha256 next to it is checked when it is there)
  --prefix DIR    install into DIR (default: ~/.local/bin)
  --no-autostart  don't start PAWPRINT at login
  --no-start      don't start the badge now
  --uninstall     remove PAWPRINT and its autostart entry (your config is kept)
  --purge         with --uninstall, also delete your config, state, and logs
  -h, --help      show this help

Environment: PAWPRINT_VERSION and PAWPRINT_PREFIX set the defaults for
--version and --prefix. PAWPRINT_DOWNLOAD_BASE replaces
https://github.com/nealhardesty/pawprint/releases (a mirror laid out the same
way: download/v<version>/<asset>, with latest redirecting to tag/v<version>).
EOF
}

say() { printf '%s\n' "$*"; }
warn() { printf 'warning: %s\n' "$*" >&2; }
die() {
    printf 'error: %s\n' "$*" >&2
    exit 1
}
has() { command -v "$1" >/dev/null 2>&1; }

# A path given as ~ or ~/..., which the calling shell did not expand
# (--prefix=~/bin, or a quoted value), with the ~ replaced by $HOME.
expand_tilde() {
    case $1 in
    \~) printf '%s\n' "$HOME" ;;
    \~/*) printf '%s/%s\n' "$HOME" "${1#\~/}" ;;
    *) printf '%s\n' "$1" ;;
    esac
}

# The absolute form of directory $1, which must exist.
absolute_dir() {
    (CDPATH='' cd -- "$1" && pwd)
}

# An XDG base directory variable's value, or $2 when it is unset, empty, or
# relative (the XDG spec says to ignore relative paths).
xdg_dir() {
    case $1 in
    /*) printf '%s\n' "$1" ;;
    *) printf '%s\n' "$2" ;;
    esac
}

# Stop the badge that binary $1 talks to, if one is running, and wait for it
# to exit. Prints "running" if it was.
stop_badge() {
    "$1" quit >/dev/null 2>&1 || return 0
    tries=0
    while "$1" status >/dev/null 2>&1 && [ $tries -lt 10 ]; do
        sleep 1
        tries=$((tries + 1))
    done
    echo running
}

uninstall() {
    if [ -x "$bin" ]; then
        "$bin" autostart off >/dev/null 2>&1 || warn "pawprint autostart off failed; removing the entry directly"
        stop_badge "$bin" >/dev/null
        rm -f "$bin"
        say "Removed $bin"
    else
        say "PAWPRINT is not installed in $prefix"
    fi
    if [ -e "$autostart_entry" ]; then
        rm -f "$autostart_entry"
        say "Removed $autostart_entry"
    fi
    if [ "$purge" = 1 ]; then
        printf '%s\n' "$data_dirs" | while IFS= read -r dir; do
            if [ -e "$dir" ]; then
                rm -rf "$dir"
                say "Deleted $dir"
            fi
        done
    else
        say "Your config and custom mascots are kept in $config_dir (--purge deletes them)."
    fi
}

download() { # url file
    if has curl; then
        curl -fsSL --retry 3 -o "$2" "$1"
    elif has wget; then
        wget -q -O "$2" "$1"
    else
        die "curl or wget is needed to download PAWPRINT"
    fi
}

# The latest release: GitHub redirects releases/latest to releases/tag/v<ver>.
latest_version() {
    if has curl; then
        url=$(curl -fsSLI -o /dev/null -w '%{url_effective}' "$base/latest") || url=
    elif has wget; then
        url=$(wget -q -S --spider "$base/latest" 2>&1 | sed -n 's/^ *[Ll]ocation: *//p' | tail -n 1 | tr -d '\r')
    else
        die "curl or wget is needed to download PAWPRINT"
    fi
    case $url in
    */tag/*) ;;
    *) die "cannot find the latest version at $base/latest; pass --version" ;;
    esac
    v=${url##*/tag/}
    v=${v%/}
    printf '%s\n' "${v#v}"
}

# Hashed from stdin: given a file name, GNU sha256sum escapes a backslash in
# it and prefixes the line with one.
sha256() {
    if has sha256sum; then
        sha256sum <"$1" | cut -d' ' -f1
    elif has shasum; then
        shasum -a 256 <"$1" | cut -d' ' -f1
    else
        die "sha256sum or shasum is needed to verify the download"
    fi
}

main() {
    version=${PAWPRINT_VERSION:-}
    prefix=${PAWPRINT_PREFIX:-}
    base=${PAWPRINT_DOWNLOAD_BASE:-$RELEASES}
    archive=
    autostart=1
    start=1
    uninstall=0
    purge=0

    while [ $# -gt 0 ]; do
        case $1 in
        --version | --archive | --prefix)
            [ $# -ge 2 ] || die "$1 needs a value"
            case $1 in
            --version) version=$2 ;;
            --archive) archive=$2 ;;
            --prefix) prefix=$2 ;;
            esac
            shift
            ;;
        --version=*) version=${1#*=} ;;
        --archive=*) archive=${1#*=} ;;
        --prefix=*) prefix=${1#*=} ;;
        --no-autostart) autostart=0 ;;
        --no-start) start=0 ;;
        --uninstall) uninstall=1 ;;
        --purge) purge=1 ;;
        -h | --help)
            usage
            exit 0
            ;;
        *) die "unknown option: $1 (see --help)" ;;
        esac
        shift
    done

    [ "$purge" = 0 ] || [ "$uninstall" = 1 ] || die "--purge goes with --uninstall"
    [ -n "${HOME:-}" ] || die "HOME is not set"
    [ -n "$prefix" ] || prefix=$HOME/.local/bin
    prefix=$(expand_tilde "$prefix")
    [ ! -d "$prefix" ] || prefix=$(absolute_dir "$prefix")
    [ -z "$archive" ] || archive=$(expand_tilde "$archive")
    version=${version#v}
    bin=$prefix/pawprint

    os=$(uname -s)
    case $os in
    Linux)
        xdg_config=$(xdg_dir "${XDG_CONFIG_HOME:-}" "$HOME/.config")
        config_dir=$xdg_config/pawprint
        data_dirs="$config_dir
$(xdg_dir "${XDG_STATE_HOME:-}" "$HOME/.local/state")/pawprint"
        autostart_entry=$xdg_config/autostart/pawprint.desktop
        case $(uname -m) in
        x86_64 | amd64) target=x86_64-unknown-linux-musl ;;
        aarch64 | arm64) target=aarch64-unknown-linux-musl ;;
        *) die "no PAWPRINT build for $(uname -m) Linux yet (x86_64 and aarch64 only)" ;;
        esac
        ;;
    Darwin)
        config_dir=$HOME/Library/Application\ Support/pawprint
        data_dirs="$config_dir
$HOME/Library/Logs/pawprint"
        autostart_entry=$HOME/Library/LaunchAgents/io.github.nealhardesty.pawprint.plist
        target=universal-apple-darwin
        ;;
    *) die "unsupported OS: $os (on Windows, use install.ps1)" ;;
    esac

    if [ "$uninstall" = 1 ]; then
        uninstall
        exit 0
    fi

    tmp=$(mktemp -d 2>/dev/null || mktemp -d -t pawprint)
    trap 'rm -rf "$tmp"' EXIT
    trap 'exit 1' HUP INT TERM PIPE

    if [ -n "$archive" ]; then
        [ -f "$archive" ] || die "no such archive: $archive"
        sums=$archive.sha256
        [ -f "$sums" ] || sums=
    else
        [ -n "$version" ] || version=$(latest_version)
        name=pawprint-$version-$target.tar.gz
        url=$base/download/v$version/$name
        say "Downloading PAWPRINT $version for $target"
        download "$url" "$tmp/$name" || die "download failed: $url"
        download "$url.sha256" "$tmp/$name.sha256" || die "download failed: $url.sha256"
        archive=$tmp/$name
        sums=$tmp/$name.sha256
    fi

    if [ -n "$sums" ]; then
        want=$(head -n 1 "$sums" | cut -d' ' -f1 | tr -d '\r' | tr 'A-F' 'a-f')
        got=$(sha256 "$archive")
        [ "$got" = "$want" ] || die "checksum mismatch for $archive (expected $want, got $got); nothing was changed"
        say "Checksum OK ($got)"
    else
        warn "no $archive.sha256 next to the archive, so its checksum is not checked"
    fi

    mkdir "$tmp/x"
    tar -xzf "$archive" -C "$tmp/x" || die "cannot unpack $archive"
    new=$tmp/x/pawprint
    [ -f "$new" ] || die "$archive has no pawprint binary in it"
    chmod 755 "$new"
    # A browser download of the archive is quarantined on macOS; curl's isn't.
    [ "$os" != Darwin ] || xattr -d com.apple.quarantine "$new" 2>/dev/null || true
    new_version=$("$new" --version 2>&1) || die "the new binary does not run on this machine: $new_version"

    # Verified. From here on the system changes.
    mkdir -p "$prefix" || die "cannot create $prefix"
    prefix=$(absolute_dir "$prefix")
    bin=$prefix/pawprint
    if [ -x "$bin" ]; then
        say "Upgrading $("$bin" --version 2>/dev/null || echo 'an older pawprint') to $new_version"
        was=$(stop_badge "$bin")
    else
        was=$(stop_badge "$new")
    fi
    if ! { cp "$new" "$bin.new" && chmod 755 "$bin.new" && mv -f "$bin.new" "$bin"; }; then
        die "cannot write $bin"
    fi
    say "Installed $new_version to $bin"

    if [ "$autostart" = 1 ]; then
        if out=$("$bin" autostart on 2>&1); then
            say "$out"
        else
            warn "could not turn on autostart: $out"
        fi
    fi

    if [ "$start" = 1 ]; then
        if out=$("$bin" 2>&1); then
            say "$out"
        else
            warn "the badge did not start: $out"
            say "(Without a desktop session, over SSH for example, that is expected: it starts at the next graphical login.)"
        fi
    elif [ -n "$was" ]; then
        say "The badge was stopped for the upgrade; start it again with: $bin"
    fi

    say ""
    "$bin" explain || true

    case :$PATH: in
    *:"$prefix":*) ;;
    *)
        say ""
        say "$prefix is not on your PATH. To run pawprint by name, add this to your shell profile:"
        say "  export PATH=\"$prefix:\$PATH\""
        ;;
    esac
}

main "$@"
