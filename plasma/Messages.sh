#!/usr/bin/env bash
# Translatable strings of the Plasma widget.
#
#   plasma/Messages.sh          extract contents/ui/*.qml,*.js into
#                               po/plasma_applet_com.agenceapi.devicehub.pot and
#                               merge it into every po/<lang>.po
#   plasma/Messages.sh --check  change nothing; fail if a string is missing,
#                               untranslated or fuzzy in a catalogue
#
# Keywords: i18n(1) i18nc(1c,2) i18np(1,2) i18ncp(1c,2,3), as KLocalizedContext;
# %1 is a KDE placeholder, not a JavaScript format.
# tests/check_i18n.py stays the gate (placeholders, obsolete entries).
set -euo pipefail
export LC_ALL=C
cd "$(dirname "$0")"
check=0
[ "${1:-}" = "--check" ] && check=1
domain=plasma_applet_com.agenceapi.devicehub
pot=po/$domain.pot
if [ $check = 1 ]; then
  tmp=$(mktemp -d "${TMPDIR:-/tmp}/akm-plasma-po.XXXXXX")
  trap 'rm -rf "$tmp"' EXIT
  pot=$tmp/$domain.pot
fi
xgettext --from-code=UTF-8 --language=JavaScript --add-comments=i18n: \
  -ki18n:1 -ki18nc:1c,2 -ki18np:1,2 -ki18ncp:1c,2,3 --no-location \
  --package-name=apple-kb-monitor \
  --msgid-bugs-address=https://github.com/litteulapi/apple-kb-monitor/issues \
  -o "$pot.new" com.agenceapi.devicehub/contents/ui/*.qml com.agenceapi.devicehub/contents/ui/*.js
sed -i -e '/^#, javascript-format$/d' -e 's/, javascript-format//' "$pot.new"
# Keep the committed POT-Creation-Date when nothing else changed, so a rerun is a no-op.
committed=po/${pot##*/}
if [ -f "$committed" ] && cmp -s <(grep -v '^"POT-Creation-Date:' "$committed") <(grep -v '^"POT-Creation-Date:' "$pot.new"); then
  [ "$committed" = "$pot" ] || cp "$committed" "$pot"; rm "$pot.new"
else
  mv "$pot.new" "$pot"
fi
rc=0
for po in po/*.po; do
  if [ $check = 1 ]; then
    merged=$tmp/$(basename "$po")
    msgmerge --quiet --no-fuzzy-matching -o "$merged" "$po" "$pot"
  else
    msgmerge --quiet --update --backup=none --no-fuzzy-matching "$po" "$pot"
    merged=$po
  fi
  stats=$(msgfmt --check --statistics -o /dev/null "$merged" 2>&1)
  echo "$po: $stats"
  if echo "$stats" | grep -qE "untranslated|fuzzy"; then rc=1; fi
done
exit $rc
