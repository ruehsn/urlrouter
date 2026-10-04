#!/usr/bin/env bash
# End-to-end test of the built app on a real Mac (run by CI after
# build-app.sh). macOS delivers a link to "URL Router.app" as an Apple Event,
# exactly as when it is the default browser; the app applies the rules, and
# a fake browser records what it was launched with.
#
# It temporarily replaces ~/Library/Application Support/URLRouter/urlrouter.txt
# and restores it afterwards, so it is safe to run on your own Mac.
set -euo pipefail
cd "$(dirname "$0")/.."

app="$PWD/dist/URL Router.app"
bin="$app/Contents/MacOS/urlrouter"
work=$(mktemp -d)
log="$work/fake.log"
cfg_dir="$HOME/Library/Application Support/URLRouter"
cfg="$cfg_dir/urlrouter.txt"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}
expect() { # expect <description> <needle> <haystack>
  if [[ "$3" != *"$2"* ]]; then
    printf 'FAIL: %s\n  expected to contain: %s\n  got: %s\n' "$1" "$2" "$3" >&2
    exit 1
  fi
  echo "ok: $1"
}
running() { pgrep -f "URL Router.app/Contents/MacOS/urlrouter" > /dev/null; }

mkdir -p "$cfg_dir"
[[ -f "$cfg" ]] && mv "$cfg" "$work/user-config.txt"
cleanup() {
  pkill -f "URL Router.app/Contents/MacOS/urlrouter" 2> /dev/null || true
  rm -f "$cfg"
  [[ -f "$work/user-config.txt" ]] && mv "$work/user-config.txt" "$cfg"
  return 0
}
trap cleanup EXIT

# A fake browser: an app bundle whose executable is a script that logs its
# arguments. It can't receive Apple Events, so the rule that uses it passes an
# extra argument, which makes URL Router launch it with the URL on the
# command line.
fake="$work/FakeBrowser.app"
mkdir -p "$fake/Contents/MacOS"
printf '#!/bin/sh\necho "$*" >> "%s"\n' "$log" > "$fake/Contents/MacOS/fake"
chmod +x "$fake/Contents/MacOS/fake"
cat > "$fake/Contents/Info.plist" << 'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>test.urlrouter.fakebrowser</string>
<key>CFBundleExecutable</key><string>fake</string>
<key>CFBundlePackageType</key><string>APPL</string>
</dict></plist>
PLIST

cat > "$cfg" << CFG
slack-team   acme  T0ACME
slack.com    slack
apple.com    safari
example.org  "$fake" --profile=Work
*            "$fake"
CFG

echo "--- rules (--test)"
out=$("$bin" --test "https://acme.slack.com/archives/C0123ABC/p1712345678123456")
expect "workspace permalink becomes a Slack deep link" \
  "open in the Slack app: slack://channel?team=T0ACME&id=C0123ABC&message=1712345678.123456" "$out"
out=$("$bin" --test "https://app.slack.com/client/T0OTHER/C0CHAN")
expect "app.slack.com link carries its own team" "slack://channel?team=T0OTHER&id=C0CHAN" "$out"
out=$("$bin" --test "https://acme.slack.com/admin")
expect "web-only Slack page falls through to the next rule" "line 5:" "$out"
out=$("$bin" --test "https://www.apple.com/mac/")
expect "built-in browser is found through LaunchServices" "Safari.app" "$out"
out=$("$bin" --test "https://example.org/x")
expect "extra arguments use a fresh launch" "run: open -n -a" "$out"
expect "extra arguments come before the URL" "--args --profile=Work https://example.org/x" "$out"

echo "--- link delivered by macOS"
url="https://example.org/e2e?x=1&y=two%20words"
open -a "$app" "$url"
for _ in $(seq 1 60); do
  [[ -s "$log" ]] && break
  sleep 0.5
done
[[ -s "$log" ]] || fail "the fake browser was never launched (no $log)"
expect "the routed link reaches the browser intact" "--profile=Work $url" "$(cat "$log")"

for _ in $(seq 1 20); do
  running || break
  sleep 0.5
done
running && fail "URL Router is still running after handling the link"
echo "ok: URL Router quits after routing"

echo "--- opened directly"
open "$app"
sleep 4
running || fail "URL Router quit instead of showing its menu when opened without a link"
echo "ok: menu is shown when opened without a link"
# Kept as a CI artifact so the dialog can be eyeballed; best-effort.
screencapture -x dist/e2e-menu.png 2> /dev/null || true

echo "All end-to-end checks passed."
