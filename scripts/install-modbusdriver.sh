#!/usr/bin/env bash
# Downloads diagslave and modpoll from modbusdriver.com into .github/tools/modbusdriver
# (kept as a CI cache path) and installs them into /usr/local/bin. Used by both the CI
# workflow and the repo justfile; a no-op when the tools are already installed.
set -euo pipefail

cd "$(dirname "$0")/.."

if command -v diagslave >/dev/null && command -v modpoll >/dev/null; then
  echo "diagslave and modpoll already installed"
  exit 0
fi

arch="$(uname -m)"
case "$arch" in
  x86_64|amd64) modbusdriver_dir="x86_64-linux-gnu" ;;
  aarch64|arm64) modbusdriver_dir="aarch64-linux-gnu" ;;
  armv7l) modbusdriver_dir="arm-linux-gnueabihf" ;;
  armv6l) modbusdriver_dir="armv6-rpi-linux-gnueabihf" ;;
  i686|i386) modbusdriver_dir="i686-linux-gnu" ;;
  *)
    echo "unsupported arch for diagslave: $arch" >&2
    exit 1
    ;;
esac

mkdir -p .github/tools/modbusdriver

for tool in diagslave modpoll; do
  if [ ! -x ".github/tools/modbusdriver/${tool}" ]; then
    extract_dir="$(mktemp -d)"
    curl -fsSL "https://www.modbusdriver.com/downloads/${tool}.tgz" -o "/tmp/${tool}.tgz"
    tar -xzf "/tmp/${tool}.tgz" -C "${extract_dir}"
    tool_path="$(find "${extract_dir}" -path "*/${modbusdriver_dir}/${tool}" -type f | head -n 1)"
    if [ -z "${tool_path}" ]; then
      echo "${tool} binary not found in archive" >&2
      exit 1
    fi
    install -m 0755 "${tool_path}" ".github/tools/modbusdriver/${tool}"
  fi
  sudo install -m 0755 ".github/tools/modbusdriver/${tool}" "/usr/local/bin/${tool}"
done
