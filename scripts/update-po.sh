#!/usr/bin/env bash
# Regenerates po/apple-kb-monitor.pot from the Rust sources and merges it into every po/*.po.
set -euo pipefail
cd "$(dirname "$0")/.."
find apihub-app -name '*.rs' -path '*/src/*' -not -path '*/target/*' | LC_ALL=C sort > po/POTFILES
xgettext -L Rust --from-code=UTF-8 -k --keyword='tr!' --keyword='trn!:1,2' --keyword='N_!' \
  --package-name=apple-kb-monitor --msgid-bugs-address=https://github.com/litteulapi/apple-kb-monitor/issues --sort-by-file --add-location=file \
  -f po/POTFILES -o po/apple-kb-monitor.pot.new
# Keep the old POT-Creation-Date when nothing else changed, so a rerun is a no-op.
if [ -f po/apple-kb-monitor.pot ] && cmp -s <(grep -v '^"POT-Creation-Date:' po/apple-kb-monitor.pot) <(grep -v '^"POT-Creation-Date:' po/apple-kb-monitor.pot.new); then
  rm po/apple-kb-monitor.pot.new
else
  mv po/apple-kb-monitor.pot.new po/apple-kb-monitor.pot
fi
for po in po/*.po; do
  [ -e "$po" ] || continue
  msgmerge --quiet --update --backup=none "$po" po/apple-kb-monitor.pot
  msgfmt --check --statistics -o /dev/null "$po"
done
