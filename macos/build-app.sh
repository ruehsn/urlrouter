#!/usr/bin/env bash
# Builds "URL Router.app" into dist/, as one universal binary for Apple
# silicon and Intel Macs, and zips it as dist/URLRouter-mac.zip.
# Runs on macOS with rustup and the Xcode command-line tools.
set -euo pipefail
cd "$(dirname "$0")/.."

export MACOSX_DEPLOYMENT_TARGET=11.0
version=$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)

for target in aarch64-apple-darwin x86_64-apple-darwin; do
  rustup target add "$target" > /dev/null
  cargo build --release --locked --target "$target"
done

app="dist/URL Router.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
lipo -create -output "$app/Contents/MacOS/urlrouter" \
  target/aarch64-apple-darwin/release/urlrouter \
  target/x86_64-apple-darwin/release/urlrouter
sed "s/@VERSION@/$version/g" macos/Info.plist > "$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist"

# Ad-hoc signature. Apple silicon won't run unsigned code at all, and signing
# the bundle rather than just the binary also seals Info.plist. It is not a
# Developer ID signature, so Gatekeeper still asks on first open (see README).
codesign --force --sign - "$app"
codesign --verify --strict "$app"

rm -f dist/URLRouter-mac.zip
ditto -c -k --keepParent "$app" dist/URLRouter-mac.zip
echo "Built $app ($version) and dist/URLRouter-mac.zip"
