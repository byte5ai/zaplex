#!/usr/bin/env bash
# Install Zaplex CLI binary on remote host for remote-server-proxy.
#
# setup.rs will replace these placeholders at runtime:
#   {download_base_url}     - e.g. https://github.com/byte5ai/zaplex/releases/latest/download
#   {install_dir}           - e.g. ~/.zap/remote-server
#   {binary_name}           - e.g. zaplex
#   {version_suffix}        - e.g. -v0.2026..., empty when no release tag
#   {staging_tarball_path}  - SCP fallback pre-uploaded tarball path, empty for normal download path
#   {expected_sha256}       - authenticated digest embedded in the client build
set -e

arch=$(uname -m)
case "$arch" in
  x86_64|amd64)  arch_name=x86_64 ;;
  aarch64|arm64) arch_name=aarch64 ;;
  *) echo "unsupported arch: $arch" >&2; exit 2 ;;
esac

os_kernel=$(uname -s)
case "$os_kernel" in
  Darwin) os_name=macos ;;
  Linux)  os_name=linux ;;
  *) echo "unsupported OS: $os_kernel" >&2; exit 2 ;;
esac

install_dir="{install_dir}"
case "$install_dir" in
  "~"|"~/"*) install_dir="${HOME}${install_dir#\~}" ;;
esac
mkdir -p "$install_dir"

tmpdir=$(mktemp -d "$install_dir/.install.XXXXXX")
# Best-effort cleanup of staging directory. Failure here should not override actual installation result:
# when trap triggers, the binary is either already moved to final path, or
# the script failed for other reasons, the latter error is more important to expose to caller.
cleanup() {
  rm -rf "$tmpdir" 2>/dev/null || true
}
trap cleanup EXIT

staging_tarball_path="{staging_tarball_path}"
if [ -n "$staging_tarball_path" ]; then
  case "$staging_tarball_path" in
    "~"|"~/"*) staging_tarball_path="${HOME}${staging_tarball_path#\~}" ;;
  esac
  mv "$staging_tarball_path" "$tmpdir/zap.tar.gz"
else
  url="{download_base_url}/zap-remote-server-$os_name-$arch_name.tar.gz"
  if command -v curl >/dev/null 2>&1; then
    curl -fSL --connect-timeout 15 "$url" -o "$tmpdir/zap.tar.gz"
  elif command -v wget >/dev/null 2>&1; then
    wget -q -O "$tmpdir/zap.tar.gz" "$url"
  else
    echo "error: neither curl nor wget is available" >&2
    exit 3
  fi
fi

expected_sha256="{expected_sha256}"
case "$expected_sha256" in
  [0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]) ;;
  *) echo "error: missing or invalid authenticated archive digest" >&2; exit 4 ;;
esac

if command -v sha256sum >/dev/null 2>&1; then
  actual_sha256=$(sha256sum "$tmpdir/zap.tar.gz" | awk '{print $1}')
elif command -v shasum >/dev/null 2>&1; then
  actual_sha256=$(shasum -a 256 "$tmpdir/zap.tar.gz" | awk '{print $1}')
else
  echo "error: neither sha256sum nor shasum is available" >&2
  exit 4
fi
if [ "$actual_sha256" != "$expected_sha256" ]; then
  echo "error: remote-server archive SHA-256 mismatch" >&2
  exit 4
fi

tar -xzf "$tmpdir/zap.tar.gz" -C "$tmpdir"

bin="$tmpdir/{binary_name}"
if [ ! -f "$bin" ]; then
  bin=$(find "$tmpdir" -type f \( -name 'zaplex' -o -name 'zap-oss' -o -name 'warp-oss' -o -name 'oz*' \) ! -path "$tmpdir/resources/*" ! -name '*.tar.gz' | head -n1)
fi
if [ -z "$bin" ]; then echo "no binary found in tarball" >&2; exit 1; fi
chmod +x "$bin"
mv "$bin" "$install_dir/{binary_name}{version_suffix}"
