#!/usr/bin/env bash
# apple-kb-monitor installer for Arch Linux / Manjaro: downloads the package of
# the latest GitHub release, checks its sha256 and installs it with pacman.
#
#   curl -fsSL https://raw.githubusercontent.com/litteulapi/apple-kb-monitor/main/install.sh | bash
#
# Or read it first: curl -fsSLO .../install.sh && less install.sh && bash install.sh
set -euo pipefail
repo=litteulapi/apple-kb-monitor
say() { printf '\033[1m==> %s\033[0m\n' "$*"; }
die() { printf 'error: %s\n' "$*" >&2; exit 1; }

# Everything runs from main, so a truncated download executes nothing.
main() {
  command -v pacman >/dev/null || die "pacman not found: this package is for Arch Linux / Manjaro"
  [ "$(uname -m)" = x86_64 ] || die "the release package is built for x86_64 only"
  for b in curl sha256sum sudo; do command -v "$b" >/dev/null || die "$b is required"; done
  [ "$(id -u)" != 0 ] || die "run it as your own user (sudo is asked when needed)"

  say "Looking for the latest release of $repo"
  api=$(curl -fsSL "https://api.github.com/repos/$repo/releases/latest") || die "no release found"
  urls=$(printf '%s\n' "$api" | grep -o '"browser_download_url": *"[^"]*"' | cut -d'"' -f4)
  pkg_url=$(printf '%s\n' "$urls" | grep '\.pkg\.tar\.zst$' | head -1)
  sum_url=$(printf '%s\n' "$urls" | grep '\.pkg\.tar\.zst\.sha256$' | head -1)
  [ -n "$pkg_url" ] && [ -n "$sum_url" ] || die "the latest release has no package or no checksum"

  work=$(mktemp -d)
  trap 'rm -rf "$work"' EXIT
  cd "$work"
  say "Downloading $(basename "$pkg_url")"
  curl -fL --progress-bar -o "$(basename "$pkg_url")" "$pkg_url"
  curl -fsSL -o "$(basename "$sum_url")" "$sum_url"
  sha256sum -c "$(basename "$sum_url")" || die "checksum mismatch: nothing installed"

  say "Installing (pacman asks for your password and confirmation)"
  # stdin is the piped script under curl | bash: pacman must ask the terminal
  sudo pacman -U --needed "$(basename "$pkg_url")" </dev/tty

  cat <<'EOF'

Done. Next steps:
  0. Enable the daemon (as your user, no sudo):
     systemctl --user enable --now apple-kb-monitord.service apple-kb-monitor-shutdown.service apple-kb-monitor-selfcheck.timer
  1. No ApiHub icon in the notification area:  systemctl --user restart plasma-plasmashell.service
  2. Check everything:  akmctl doctor
Uninstall: sudo pacman -R apple-kb-monitor
EOF
}

main "$@"
