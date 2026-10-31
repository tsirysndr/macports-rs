#!/bin/sh
# Install port-rs from its GitHub releases:
#
#   curl -fsSL https://raw.githubusercontent.com/tsirysndr/macports-rs/main/install.sh | sh
#
# Environment:
#   PORT_RS_VERSION      release tag to install (default: the latest release)
#   PORT_RS_INSTALL_DIR  where to put `port` (default: /usr/local/bin)
#   PORT_RS_DOWNLOAD_URL base URL holding the release assets (overrides the above)
set -eu

repo="tsirysndr/macports-rs"
install_dir="${PORT_RS_INSTALL_DIR:-/usr/local/bin}"

fail() {
    echo "install.sh: $*" >&2
    exit 1
}

[ "$(uname -s)" = "Darwin" ] || fail "port-rs supports macOS only"
case "$(uname -m)" in
    arm64 | aarch64) target="aarch64-apple-darwin" ;;
    x86_64)
        # A shell running under Rosetta on Apple Silicon still gets the native build.
        if [ "$(sysctl -in sysctl.proc_translated 2>/dev/null)" = "1" ]; then
            target="aarch64-apple-darwin"
        else
            target="x86_64-apple-darwin"
        fi
        ;;
    *) fail "unsupported architecture $(uname -m)" ;;
esac

if [ -n "${PORT_RS_DOWNLOAD_URL:-}" ]; then
    base="$PORT_RS_DOWNLOAD_URL"
elif [ -n "${PORT_RS_VERSION:-}" ]; then
    base="https://github.com/$repo/releases/download/$PORT_RS_VERSION"
else
    base="https://github.com/$repo/releases/latest/download"
fi
asset="port-$target.tar.gz"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT INT TERM

echo "Downloading $asset..."
curl -fsSL -o "$work/$asset" "$base/$asset" || fail "cannot download $base/$asset"
curl -fsSL -o "$work/$asset.sha256" "$base/$asset.sha256" ||
    fail "cannot download $base/$asset.sha256"
(cd "$work" && shasum -a 256 -c "$asset.sha256" >/dev/null) ||
    fail "checksum verification failed for $asset"
tar -xzf "$work/$asset" -C "$work" port

sudo=""
if [ ! -d "$install_dir" ] || [ ! -w "$install_dir" ]; then
    if [ "$(id -u)" -ne 0 ]; then
        command -v sudo >/dev/null || fail "$install_dir is not writable and sudo is unavailable"
        sudo="sudo"
        echo "Installing to $install_dir requires administrator privileges."
    fi
fi
$sudo mkdir -p "$install_dir"
$sudo install -m 0755 "$work/port" "$install_dir/port"
# The same binary acts as portindex when invoked under that name.
$sudo ln -sf port "$install_dir/portindex"

echo "Installed $("$install_dir/port" --version) to $install_dir/port"
case ":$PATH:" in
    *":$install_dir:"*) ;;
    *) echo "Add $install_dir to your PATH to use port." ;;
esac
