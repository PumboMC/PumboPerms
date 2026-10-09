#!/usr/bin/env bash
# Builds PumboPerms for every supported Pumpkin release into dist/ at the
# workspace root:
#   dist/PumboPerms-26.3.wasm  -> Pumpkin 0.2.0+26.3-26.51     (Minecraft 26.3)
#   dist/PumboPerms-26.2.wasm  -> Pumpkin 0.1.0-dev+26.2-26.45 (Minecraft 26.2)
set -euo pipefail
cd "$(dirname "$0")/../.."
TARGET=wasm32-wasip2
PKG=pumbo-perms-pumpkin
OUT=pumbo_perms_pumpkin.wasm
mkdir -p dist

cargo build -p "$PKG" --release --target "$TARGET"
cp "target/$TARGET/release/$OUT" dist/PumboPerms-26.3.wasm

cargo build -p "$PKG" --release --target "$TARGET" --no-default-features --features mc262 --target-dir target/mc262
cp "target/mc262/$TARGET/release/$OUT" dist/PumboPerms-26.2.wasm

ls -l dist/PumboPerms-*.wasm
