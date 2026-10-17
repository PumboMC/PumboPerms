#!/usr/bin/env bash
# Builds PumboPerms for Pumpkin into dist/ at the workspace root:
#   dist/PumboPerms-26.3.wasm  -> Pumpkin 0.2.0+26.3-26.51 (Minecraft 26.3)
# The code for Pumpkin 0.1.0-dev (Minecraft 26.2, feature mc262) stays, but its
# builds are paused for now.
set -euo pipefail
cd "$(dirname "$0")/../.."
TARGET=wasm32-wasip2
PKG=pumbo-perms-pumpkin
OUT=pumbo_perms_pumpkin.wasm
mkdir -p dist

cargo build -p "$PKG" --release --target "$TARGET"
cp "target/$TARGET/release/$OUT" dist/PumboPerms-26.3.wasm

ls -l dist/PumboPerms-*.wasm
