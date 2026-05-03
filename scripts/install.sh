#!/usr/bin/env sh
set -eu

repo="sderosiaux/launchdeck"
version="${LAUNCHDECK_VERSION:-latest}"
bin_dir="${BIN_DIR:-$HOME/.local/bin}"
verify=1

usage() {
  cat <<'EOF'
Install Launchdeck.

Usage:
  install.sh [--version v0.1.0] [--bin-dir ~/.local/bin] [--no-verify]

Environment:
  LAUNCHDECK_VERSION  Version to install, or "latest"
  BIN_DIR             Destination directory
EOF
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --version)
      if [ "$#" -lt 2 ]; then
        echo "--version needs a value" >&2
        exit 1
      fi
      version="$2"
      shift 2
      ;;
    --bin-dir)
      if [ "$#" -lt 2 ]; then
        echo "--bin-dir needs a value" >&2
        exit 1
      fi
      bin_dir="$2"
      shift 2
      ;;
    --no-verify)
      verify=0
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "unknown option: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

need() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing required command: $1" >&2
    exit 1
  fi
}

need curl
need tar
need shasum
need install

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

asset="launchdeck-${target}.tar.gz"
if [ "$version" = "latest" ]; then
  base_url="https://github.com/${repo}/releases/latest/download"
else
  base_url="https://github.com/${repo}/releases/download/${version}"
fi

tmp_dir="$(mktemp -d)"
trap 'rm -rf "$tmp_dir"' EXIT INT TERM

curl -fsSL "${base_url}/${asset}" -o "$tmp_dir/$asset"

if [ "$verify" -eq 1 ]; then
  curl -fsSL "${base_url}/checksums.txt" -o "$tmp_dir/checksums.txt"
  if ! grep "  ${asset}$" "$tmp_dir/checksums.txt" > "$tmp_dir/checksum.txt"; then
    echo "checksum for ${asset} not found in release checksums.txt" >&2
    exit 1
  fi
  (cd "$tmp_dir" && shasum -a 256 -c checksum.txt)
fi

mkdir -p "$bin_dir"
tar -xzf "$tmp_dir/$asset" -C "$tmp_dir"
install -m 755 "$tmp_dir/launchdeck-${target}/launchdeck" "$bin_dir/launchdeck"

echo "installed launchdeck to $bin_dir/launchdeck"
case ":$PATH:" in
  *":$bin_dir:"*) ;;
  *) echo "note: $bin_dir is not in PATH" ;;
esac
