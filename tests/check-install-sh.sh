#!/usr/bin/env bash
# install.sh run as `curl … | bash` on a terminal: pacman must read the terminal,
# not the piped script, and the text after pacman must still be printed.
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
command -v script >/dev/null || { echo "check-install-sh: util-linux script not installed, skipped"; exit 0; }
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
mkdir "$tmp/bin"
stub() { printf '#!/usr/bin/env bash\n%s\n' "$2" > "$tmp/bin/$1"; chmod +x "$tmp/bin/$1"; }
stub uname 'echo x86_64'
stub id 'echo 1000'
stub sudo 'exec "$@"'
stub pacman 'if [ -t 0 ]; then echo "PACMAN_STDIN=tty"; else echo "PACMAN_STDIN=pipe"; cat >/dev/null; fi'
stub curl '
pkg=apple-kb-monitor-1-1-x86_64.pkg.tar.zst
for a; do case $a in
  *api.github.com*) printf "{\"browser_download_url\": \"https://x/%s\", \"browser_download_url\": \"https://x/%s.sha256\"}\n" "$pkg" "$pkg"; exit 0 ;;
esac; done
case ${*: -1} in
  *.sha256) printf fake | sha256sum | sed "s/-\$/$pkg/" > "$pkg.sha256" ;;
  *) printf fake > "$pkg" ;;
esac'

out=$(script -qec "cat '$here/install.sh' | env PATH='$tmp/bin:$PATH' bash" /dev/null </dev/null 2>&1 | tr -d '\r') || true
fail() { echo "FAIL check-install-sh: $1" >&2; printf '%s\n' "$out" | tail -15 >&2; exit 1; }
grep -q 'PACMAN_STDIN=tty' <<<"$out" || fail "pacman did not read the terminal"
grep -q 'Done. Next steps' <<<"$out" || fail "the text after pacman was swallowed"
echo "install.sh piped into bash: pacman reads the terminal, next steps printed"
