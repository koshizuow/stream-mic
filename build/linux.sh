#!/bin/sh
# Run on the build machine from the repo root: build the workspace inside the Ubuntu 24.04 image.
# Output: target-u2404/release/. The cargo registry is cached in ~/.cache/stream-mic-cargo.
set -e
podman image exists stream-mic-build || podman build -t stream-mic-build build/
mkdir -p "$HOME/.cache/stream-mic-cargo"
podman run --rm -v "$PWD:/src" -v "$HOME/.cache/stream-mic-cargo:/opt/cargo/registry" \
  -e CARGO_TARGET_DIR=/src/target-u2404 stream-mic-build cargo build --release "$@"
