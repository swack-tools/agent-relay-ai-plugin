#!/usr/bin/env bash
# Build Relay Map: the hub, the WebAssembly front end, and collector binaries
# for remote computers (bin/relay-map-<os>-<arch>).
set -euo pipefail
cd "$(dirname "$0")"

echo "== front end (wasm32)"
cargo build -p relay-map-web --target wasm32-unknown-unknown --release
rm -rf dist && mkdir -p dist/pkg
wasm-bindgen --target web --no-typescript --out-dir dist/pkg \
  target/wasm32-unknown-unknown/release/relay_map_web.wasm
cp web/index.html web/style.css dist/

echo "== hub / collector (native)"
cargo build -p relay-map --release
mkdir -p bin
case "$(uname -sm)" in
  "Darwin arm64") cp target/release/relay-map bin/relay-map-macos-aarch64 ;;
  "Darwin x86_64") cp target/release/relay-map bin/relay-map-macos-x86_64 ;;
  "Linux x86_64") cp target/release/relay-map bin/relay-map-linux-x86_64 ;;
  "Linux aarch64") cp target/release/relay-map bin/relay-map-linux-aarch64 ;;
esac

# Static Linux collectors for remote servers, when cargo-zigbuild is available.
if [[ "${SKIP_CROSS:-}" != 1 ]] && command -v cargo-zigbuild >/dev/null; then
  for t in x86_64-unknown-linux-musl aarch64-unknown-linux-musl; do
    if rustup target list --installed | grep -qx "$t"; then
      echo "== collector for $t"
      cargo zigbuild -p relay-map --release --target "$t"
      arch="${t%%-*}"
      cp "target/$t/release/relay-map" "bin/relay-map-linux-$arch"
    fi
  done
fi
echo "done: run ./target/release/relay-map serve"
