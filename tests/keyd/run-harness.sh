#!/usr/bin/env bash
# tests/keyd/run-harness.sh - keyd 2.6.0 reload use-after-free harness (ASAN).
#
# Builds tests/keyd/reload-harness.c against the official keyd 2.6.0 sources
# (sha256-pinned, the tarball Arch's keyd 2.6.0-5 is built from), twice:
#   vanilla  daemon.c as released        -> the `mouse` and `timeout`
#            scenarios MUST be caught by ASAN (proves the upstream bug is
#            still the one we documented; a pass means the harness rotted)
#   patched  + keyd-2.6.0-reload-active_kbd.patch -> every scenario MUST pass
# and runs them against keyd/apple-keyboard.conf (the file we ship).
#
# Never opens /dev/uinput or /dev/input, needs no root, grabs nothing.
#
# usage: tests/keyd/run-harness.sh [--src DIR]
#   --src DIR   use an already extracted keyd-2.6.0 tree (offline)
#   env KEYD_SRC_CACHE  where the tarball is cached
#                       (default ${XDG_CACHE_HOME:-~/.cache}/apple-kb-monitor)
# exit: 0 ok, 1 failure, 77 skipped (no cc / no sources and no network)
set -u

here=$(cd "$(dirname "$0")" && pwd)
top=$(cd "$here/../.." && pwd)
conf="$top/keyd/apple-keyboard.conf"
ver=2.6.0
sha=697089681915b89d9e98caf93d870dbd4abce768af8a647d54650a6a90744e26
url="https://github.com/rvaiya/keyd/archive/refs/tags/v$ver.tar.gz"

src=""
if [ "${1:-}" = "--src" ]; then src=$2; fi

command -v cc >/dev/null 2>&1 || { echo "SKIP: no C compiler"; exit 77; }

work=$(mktemp -d "${TMPDIR:-/tmp}/keyd-harness.XXXXXX")
trap 'rm -rf "$work"' EXIT

if [ -z "$src" ]; then
  cache=${KEYD_SRC_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/apple-kb-monitor}
  tgz="$cache/keyd-$ver.tar.gz"
  if [ ! -f "$tgz" ]; then
    mkdir -p "$cache"
    if ! curl -fsSL -o "$tgz.part" "$url"; then
      rm -f "$tgz.part"; echo "SKIP: keyd $ver sources unavailable (offline?)"; exit 77
    fi
    mv "$tgz.part" "$tgz"
  fi
  echo "$sha  $tgz" | sha256sum -c --quiet - || { echo "FAIL: $tgz sha256 mismatch"; exit 1; }
  tar -xzf "$tgz" -C "$work"
  src="$work/keyd-$ver"
fi
[ -f "$src/src/daemon.c" ] || { echo "FAIL: $src is not a keyd source tree"; exit 1; }

build() { # NAME SRC_DIR
  local out="$work/h-$1" s="$2/src" f srcs=()
  for f in "$s"/*.c; do
    case ${f##*/} in daemon.c|keyd.c|device.c|evloop.c|monitor.c|check.c) ;; *) srcs+=("$f") ;; esac
  done
  cc -g -O1 -std=c11 -D_DEFAULT_SOURCE -fno-omit-frame-pointer \
     -fsanitize=address,undefined -fno-sanitize-recover=undefined \
     -DVERSION='"v2.6.0 harness"' -DSOCKET_PATH='"/nonexistent/keyd.socket"' \
     -DDATA_DIR='"/usr/share/keyd"' -iquote "$s" \
     "$here/reload-harness.c" "${srcs[@]}" "$s/vkbd/stdout.c" -lpthread -o "$out" \
     > "$work/build-$1.log" 2>&1 || { cat "$work/build-$1.log" | tail -20; echo "FAIL: build $1"; exit 1; }
}

cp -r "$src" "$work/patched"
patch -s -d "$work/patched" -p1 < "$here/keyd-2.6.0-reload-active_kbd.patch" || { echo "FAIL: patch"; exit 1; }
build vanilla "$src"
build patched "$work/patched"

mkdir -p "$work/conf.d"
cp "$conf" "$work/conf.d/"

rc=0
run() { # BUILD SCENARIO EXPECT(pass|asan)
  local log="$work/run-$1-$2.log" got
  ASAN_OPTIONS=detect_leaks=0:abort_on_error=0 UBSAN_OPTIONS=print_stacktrace=1 \
    timeout 30 "$work/h-$1" "$work/conf.d" "$2" > "$log" 2>&1
  local st=$?
  if [ $st -eq 0 ]; then got=pass
  elif grep -q "heap-use-after-free\|SEGV on unknown address" "$log"; then got=asan
  else got="exit$st"; fi
  if [ "$got" = "$3" ]; then
    printf 'ok    %-8s %-8s %s\n' "$1" "$2" "$got"
  else
    printf 'FAIL  %-8s %-8s got %s, expected %s\n' "$1" "$2" "$got" "$3"; tail -30 "$log"; rc=1
  fi
  if [ "$got" = asan ] && [ -n "${KEYD_HARNESS_SHOW:-}" ]; then
    grep -m1 -A12 "ERROR: AddressSanitizer" "$log"
  fi
}

run vanilla parse   pass
run vanilla rekey   pass
run vanilla mouse   asan
run vanilla timeout asan
run patched parse   pass
run patched rekey   pass
run patched mouse   pass
run patched timeout pass

[ $rc -eq 0 ] && echo "keyd $ver harness OK (bug reproduced on vanilla, absent once patched; $conf parses under ASAN)"
exit $rc
