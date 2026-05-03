#!/usr/bin/env sh
set -eu

repo="sderosiaux/launchdeck"
version="${LAUNCHDECK_VERSION:-latest}"
bin_dir="${BIN_DIR:-$HOME/.local/bin}"

case "$(uname -s)" in
  Darwin) ;;
  *)
    echo "launchdeck currently ships release binaries for macOS only." >&2
    exit 1
    ;;
esac

case "$(uname -m)" in
  arm64) target="aarch64-apple-darwin" ;;
  x86_64) target="x86_64-apple-darwin" ;;
  *)
    echo "unsupported macOS architecture: $(uname -m)" >&2
    exit 1
    ;;
esac

if [ "$version" = "latest" ]; then
  url="https://github.com/${repo}/releases/latest/download/launchdeck-${target}.tar.gz"
else
  url="https://github.com/${repo}/releases/download/${version}/launchdeck-${target}.tar.gz"
fi

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT INT TERM

mkdir -p "$bin_dir"
curl -fsSL "$url" -o "$tmp_dir/launchdeck.tar.gz"
tar -xzf "$tmp_dir/launchdeck.tar.gz" -C "$tmp_dir"
install -m 755 "$tmp_dir/launchdeck-${target}/launchdeck" "$bin_dir/launchdeck"

echo "installed launchdeck to $bin_dir/launchdeck"
case ":$PATH:" in
  *":$bin_dir:"*) ;;
  *) echo "note: $bin_dir is not in PATH" ;;
esac
