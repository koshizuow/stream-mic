#!/bin/sh
# Run on the build machine from the repo root: build the app (engine, window and tray in one program) inside the Ubuntu 24.04
# image. Output: target-u2404/release/stream-mic-app is not used: app/ is its own package named stream-mic, so the
# program is target-u2404/app/release/stream-mic.
set -e
podman image exists stream-mic-build || podman build -t stream-mic-build build/
mkdir -p "$HOME/.cache/stream-mic-cargo"
podman run --rm -v "$PWD:/src" -v "$HOME/.cache/stream-mic-cargo:/opt/cargo/registry" \
  -e CARGO_TARGET_DIR=/src/target-u2404/app stream-mic-build cargo build --release --manifest-path app/Cargo.toml "$@"
