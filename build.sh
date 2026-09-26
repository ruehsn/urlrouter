#!/usr/bin/env sh
# Cross-compiles urlrouter.exe from Linux/macOS/WSL into dist/.
# Needs: rustup, and the MinGW-w64 linker (Debian/Ubuntu: apt install gcc-mingw-w64-x86-64).
# On Windows itself, `cargo build --release` is enough (see README).
set -eu
cd "$(dirname "$0")"
rustup target add x86_64-pc-windows-gnu >/dev/null
cargo test
cargo build --release --target x86_64-pc-windows-gnu
mkdir -p dist
cp target/x86_64-pc-windows-gnu/release/urlrouter.exe dist/
echo "Built dist/urlrouter.exe"
