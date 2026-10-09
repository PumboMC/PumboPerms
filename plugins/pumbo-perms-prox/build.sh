#!/usr/bin/env bash
# Builds PumboPerms for PumboProx into dist/pumbo-perms.wasm: one file to drop
# into the proxy's plugins/. The manifest (pumbo-perms.yml) and the default
# config (assets/config.yml) are built in; the proxy writes
# plugins/pumbo-perms/config.yml at the first start.
set -euo pipefail
cd "$(dirname "$0")"
ROOT=$(cd ../.. && pwd)
TARGET=wasm32-wasip2
mkdir -p dist
cargo build --release --target "$TARGET" -p pumbo-perms-prox --target-dir "$ROOT/target/perms-prox"
cp "$ROOT/target/perms-prox/$TARGET/release/pumbo_perms_prox.wasm" dist/pumbo-perms.wasm
ls -l dist/pumbo-perms.wasm
