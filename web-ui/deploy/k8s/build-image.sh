#!/usr/bin/env bash
# Build the Relay Map container image on a Mac (Apple silicon) and load it into
# a single-node cluster's containerd over ssh.
#
#   deploy/k8s/build-image.sh <ssh-host-of-the-node> [tag]
#
# Builds in a separate cargo target folder, so a hub already running from
# target/release is never overwritten. The image holds the static Linux hub,
# the WebAssembly front end and collectors for Linux and macOS computers.
set -euo pipefail
node="${1:?usage: build-image.sh <ssh-host-of-the-node> [tag]}"
here="$(cd "$(dirname "$0")" && pwd)"
ws="$(cd "$here/../.." && pwd)"
tag="${2:-$(git -C "$ws" rev-parse --short HEAD)}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ws/target/image}"
ctx="$CARGO_TARGET_DIR/context"
rm -rf "$ctx" && mkdir -p "$ctx/dist/pkg" "$ctx/bin"

cd "$ws"
echo "== front end (wasm32)"
cargo build -p relay-map-web --target wasm32-unknown-unknown --release
wasm-bindgen --target web --no-typescript --out-dir "$ctx/dist/pkg" \
  "$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/relay_map_web.wasm"
cp web/index.html web/style.css "$ctx/dist/"

echo "== hub and collectors"
for t in x86_64-unknown-linux-musl aarch64-unknown-linux-musl; do
  cargo zigbuild -p relay-map --release --target "$t"
  cp "$CARGO_TARGET_DIR/$t/release/relay-map" "$ctx/bin/relay-map-linux-${t%%-*}"
done
cargo build -p relay-map --release --target aarch64-apple-darwin
cp "$CARGO_TARGET_DIR/aarch64-apple-darwin/release/relay-map" "$ctx/bin/relay-map-macos-aarch64"
cp "$here/Dockerfile" "$here/relay-send-via-ssh.py" "$ctx/"

echo "== image relay-map:$tag on $node"
COPYFILE_DISABLE=1 tar --no-xattrs -C "$ctx" -czf - . | ssh "$node" "set -e; d=\$(mktemp -d); tar -C \$d -xzf -;
  docker build -q -t relay-map:$tag \$d;
  docker save relay-map:$tag | sudo ctr -a ${CONTAINERD_SOCK:-/run/containerd/containerd.sock} -n k8s.io images import -;
  rm -rf \$d"
echo "done: set images[relay-map].newTag to $tag in your overlay and kubectl apply -k it"
