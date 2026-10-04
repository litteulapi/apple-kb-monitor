#!/usr/bin/env bash
# The three catalog generators, run twice on a copy of the tree, must leave every catalog byte for byte as committed.
set -euo pipefail
export LC_ALL=C
top=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/akm-po-stable.XXXXXX")
trap 'rm -rf "$work"' EXIT
cd "$top"
{ git ls-files -z --cached; git ls-files -z --others --exclude-standard; } \
  | xargs -0 -I{} sh -c '[ -f "{}" ] && echo "{}"' \
  | grep -E '^(po/|scripts/update-po\.sh$|kcm/(Messages\.sh|ui/|po/)|plasma/(Messages\.sh|po/|com\.agenceapi\.devicehub/contents/ui/)|apihub-app/.*\.rs$)' \
  | tar -cf - -T - | tar -C "$work" -xf -
for run in 1 2; do
  for gen in scripts/update-po.sh kcm/Messages.sh plasma/Messages.sh; do
    (cd "$work" && bash "$gen" > "$work/gen.log" 2>&1) || { cat "$work/gen.log"; echo "FAIL: $gen (run $run)"; exit 1; }
  done
done
rc=0
for d in po kcm/po plasma/po; do
  diff -r -q "$top/$d" "$work/$d" || rc=1
done
if [ $rc != 0 ]; then
  for d in po kcm/po plasma/po; do diff -r -u "$top/$d" "$work/$d"; done | head -40 || true
  echo "FAIL: a catalog drifts when regenerated: run scripts/update-po.sh, kcm/Messages.sh, plasma/Messages.sh and commit"
  exit 1
fi
echo "catalogs stable: three generators, two runs, no diff"
