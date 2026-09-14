#!/bin/sh
# Corral installer — https://github.com/Poordeveloper/corral
#
#   curl -fsSL https://raw.githubusercontent.com/Poordeveloper/corral/main/install.sh | sh
#
# Places the latest published release for this machine, links `corral` into
# ~/.local/bin, and enables Corral's integration with the coding agents it
# finds — after saying which. Install is not upgrade: an existing
# installation is refused, and `corral uninstall` is how it goes away.
#
#   CORRAL_VERSION=vX.Y.Z     install that release instead of the latest
#   CORRAL_INSTALL_FROM=<dir> install from a local scripts/package output
#                             (the archive and its SHA256SUMS) instead of
#                             downloading; the checksum is still verified
#
# Served from main and never frozen at a tag (grill Q17): CORRAL_VERSION pins
# the artifact, not this file. POSIX sh: nothing here needs bash.
set -eu

REPO="Poordeveloper/corral"

say() { printf '%s\n' "$*"; }
die() { printf 'install: %s\n' "$*" >&2; exit 1; }

os="$(uname -s)"
arch="$(uname -m)"
case "$os/$arch" in
    Darwin/arm64) platform="macos-arm64" ;;
    Linux/x86_64) platform="linux-x86_64" ;;
    *) die "unsupported platform $os $arch; Corral runs on macOS (Apple Silicon) and Ubuntu 24.04 (x86_64)" ;;
esac

# The installer's home. The installed binaries resolve the account home from
# the account database rather than from $HOME (ADR 0001 D1); in every
# supported setup those are the same directory, and the package smoke keeps
# them equal where it substitutes one.
home="${HOME:?HOME is not set}"
case "$platform" in
    macos-arm64)
        target="$home/Applications/Corral.app"
        executables="$target/Contents/MacOS"
        ;;
    linux-x86_64)
        target="$home/.local/share/corral"
        executables="$target/bin"
        ;;
esac
link="$home/.local/bin/corral"

# Before anything is fetched. Install is not upgrade (M1 packaging plan D4):
# what to do about a live daemon and its state is upgrade's own design.
if [ -e "$target" ] || [ -L "$target" ]; then
    die "Corral is already installed at $target; run \`corral uninstall\` first — upgrading in place is not part of M1"
fi

for tool in tar mktemp; do
    command -v "$tool" >/dev/null 2>&1 || die "$tool is needed and was not found"
done
if [ "$platform" = "macos-arm64" ]; then
    command -v ditto >/dev/null 2>&1 || die "ditto is needed and was not found"
fi

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d' ' -f1
    else
        die "neither sha256sum nor shasum was found; the download cannot be verified"
    fi
}

tmp="${TMPDIR:-/tmp}"
workdir="$(mktemp -d "${tmp%/}/corral-install.XXXXXX")"
cleanup() { rm -rf "$workdir"; }
trap cleanup EXIT INT TERM

# Resolve the release: a local scripts/package output, a pinned tag, or the
# latest published (non-draft, non-prerelease) release.
if [ -n "${CORRAL_INSTALL_FROM:-}" ]; then
    from="$CORRAL_INSTALL_FROM"
    [ -f "$from/SHA256SUMS" ] || die "$from/SHA256SUMS not found; CORRAL_INSTALL_FROM names a scripts/package output"
    case "$platform" in
        macos-arm64) pattern='Corral-*-macos-arm64.zip' ;;
        linux-x86_64) pattern='corral-*-linux-x86_64.tar.gz' ;;
    esac
    asset=""
    for candidate in "$from"/$pattern; do
        [ -f "$candidate" ] || continue
        [ -z "$asset" ] || die "$from holds more than one $platform archive"
        asset="$(basename "$candidate")"
    done
    [ -n "$asset" ] || die "$from holds no $platform archive"
    version="${asset#*-}"
    version="${version%-$platform.*}"
    cp "$from/$asset" "$workdir/$asset"
    cp "$from/SHA256SUMS" "$workdir/SHA256SUMS"
    say "Installing Corral $version from $from"
else
    command -v curl >/dev/null 2>&1 || die "curl is needed and was not found"
    if [ -n "${CORRAL_VERSION:-}" ]; then
        tag="$CORRAL_VERSION"
    else
        curl -fsSL -H 'Accept: application/vnd.github+json' -o "$workdir/latest.json" \
            "https://api.github.com/repos/$REPO/releases/latest" \
            || die "the latest Corral release could not be looked up"
        tag="$(sed -n 's/^[[:space:]]*"tag_name":[[:space:]]*"\([^"]*\)".*/\1/p' "$workdir/latest.json" | head -n 1)"
        [ -n "$tag" ] || die "no published Corral release was found"
    fi
    version="${tag#v}"
    case "$platform" in
        macos-arm64) asset="Corral-$version-macos-arm64.zip" ;;
        linux-x86_64) asset="corral-$version-linux-x86_64.tar.gz" ;;
    esac
    base="https://github.com/$REPO/releases/download/$tag"
    say "Installing Corral $version from $base"
    curl -fsSL -o "$workdir/$asset" "$base/$asset" || die "$asset could not be downloaded from $base"
    curl -fsSL -o "$workdir/SHA256SUMS" "$base/SHA256SUMS" || die "SHA256SUMS could not be downloaded from $base"
fi

# Integrity, before a byte is extracted. A manifest from the same release is
# integrity against a damaged download, not an independent trust root.
expected="$(sed -n "s/^\([0-9a-f]\{64\}\)[[:space:]]\{1,\}\*\{0,1\}$asset\$/\1/p" "$workdir/SHA256SUMS" | head -n 1)"
[ -n "$expected" ] || die "SHA256SUMS does not name $asset; nothing was installed"
actual="$(sha256_of "$workdir/$asset")"
[ "$expected" = "$actual" ] || die "checksum mismatch for $asset (expected $expected, got $actual); nothing was installed"

# Unpack beside the download, then move the finished tree into place, so an
# interrupted extraction never leaves half an installation at the target.
mkdir "$workdir/unpacked"
case "$platform" in
    macos-arm64)
        ditto -x -k "$workdir/$asset" "$workdir/unpacked"
        [ -x "$workdir/unpacked/Corral.app/Contents/MacOS/corral" ] || die "$asset does not hold Corral.app"
        mkdir -p "$home/Applications"
        mv "$workdir/unpacked/Corral.app" "$target"
        ;;
    linux-x86_64)
        tar -xzf "$workdir/$asset" -C "$workdir/unpacked"
        tree="$workdir/unpacked/corral-$version-linux-x86_64"
        [ -x "$tree/corral" ] || die "$asset does not hold corral"
        mkdir -p "$workdir/tree"
        mv "$tree" "$workdir/tree/bin"
        mkdir -p "$home/.local/share"
        mv "$workdir/tree" "$target"
        ;;
esac
say "Placed $target"

# The CLI on PATH. A link is replaced; anything else at that name is the
# user's and is left alone.
mkdir -p "$home/.local/bin"
if [ -e "$link" ] && [ ! -L "$link" ]; then
    say "install: $link exists and is not a symlink; it was left alone — the installed CLI is $executables/corral"
else
    ln -sfn "$executables/corral" "$link"
    say "Linked $link"
fi
case ":${PATH:-}:" in
    *":$home/.local/bin:"*) ;;
    *)
        say ""
        say "~/.local/bin is not on your PATH. Add this to your shell profile:"
        say "    export PATH=\"\$HOME/.local/bin:\$PATH\""
        ;;
esac

# The agents this machine has, by the configuration they keep or the
# command they install.
providers=""
if [ -d "$home/.claude" ] || command -v claude >/dev/null 2>&1; then
    providers="claude"
fi
if [ -d "$home/.codex" ] || command -v codex >/dev/null 2>&1; then
    providers="${providers:+$providers }codex"
fi

if [ -z "$providers" ]; then
    say ""
    say "No supported coding agent was found (Claude Code, Codex); nothing was integrated."
    say "After installing one: corral integration enable <claude|codex>"
else
    say ""
    say "Corral will enable its integration with: $providers"
    say "This adds Corral's hook entries to the agent's own user configuration"
    say "(~/.claude/settings.json, ~/.codex/config.toml) so its sessions report to"
    say "Corral. Nothing else in those files is changed, and corrald is the only"
    say "thing that writes them."
    say ""
    for provider in $providers; do
        if ! "$executables/corral" integration enable "$provider"; then
            say "install: the $provider integration was not enabled (see above). Corral is installed;"
            say "         after resolving it: corral integration enable $provider"
        fi
    done
fi

say ""
say "Installed $("$executables/corral" --version) at $target"
say "To remove it: corral uninstall"
