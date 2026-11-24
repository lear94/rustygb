#!/usr/bin/env bash
# Opens the desktop window on a virtual display, plays a ROM that writes battery RAM, and
# checks that SIGTERM ends it cleanly with the game saved.
#
# Needs Xvfb and a Vulkan driver; a software one such as lavapipe is enough.
set -euo pipefail

# Software rendering on a loaded runner is slow to open the first window.
STARTUP_TIMEOUT_S=60
EXIT_TIMEOUT_S=20
# First battery byte the ROM stores.
EXPECTED_SAVE=5a

binary="${1:?usage: window_smoke.sh <binary>}"
work="$(mktemp -d)"
trap 'kill -9 "${pid:-}" 2>/dev/null || true; rm -rf "$work"' EXIT

python3 - "$work/game.gb" <<'PYTHON'
import sys

# Enable RAM, store 0x5A at 0xA000, loop.
program = bytes.fromhex("3e0a ea0000 3e5a ea00a0 18fe")
rom = bytearray(0x8000)
rom[0x100:0x103] = bytes.fromhex("c35001")
rom[0x147] = 0x03
rom[0x149] = 0x02
rom[0x150:0x150 + len(program)] = program
open(sys.argv[1], "wb").write(rom)
PYTHON

"$binary" "$work/game.gb" 2>"$work/stderr" &
pid=$!

fail() {
  echo "$1"
  cat "$work/stderr"
  exit 1
}

window_open() { xwininfo -root -tree | grep -q "RustyGB - game.gb"; }
for ((i = 0; i < STARTUP_TIMEOUT_S; i++)); do
  kill -0 "$pid" 2>/dev/null || fail "exited before opening a window"
  if window_open; then break; fi
  sleep 1
done
window_open || fail "no window after ${STARTUP_TIMEOUT_S}s"
sleep 2

kill -TERM "$pid"
for ((i = 0; i < EXIT_TIMEOUT_S; i++)); do
  kill -0 "$pid" 2>/dev/null || break
  sleep 1
done
kill -0 "$pid" 2>/dev/null && fail "still running ${EXIT_TIMEOUT_S}s after SIGTERM"
status=0
wait "$pid" || status=$?
[ "$status" -eq 0 ] || fail "exit status $status"
saved="$(head -c1 "$work/game.sav" | od -An -tx1 | tr -d ' ')"
[ "$saved" = "$EXPECTED_SAVE" ] || fail "saved '$saved', expected $EXPECTED_SAVE"
echo "window opened, SIGTERM saved the game"
