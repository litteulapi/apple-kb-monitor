#!/usr/bin/env bash
# Translatable strings of the KCM (#250).
#
#   kcm/Messages.sh          extract ui/*.qml into po/kcm_applekeyboard.pot and
#                            merge it into every po/<lang>/kcm_applekeyboard.po
#   kcm/Messages.sh --check  change nothing; fail if a string of ui/*.qml is
#                            missing, untranslated or fuzzy in a catalogue
#
# Keywords: i18n(1) i18nc(1c,2) i18np(1,2) i18ncp(1c,2,3), as KLocalizedContext.
set -euo pipefail
export LC_ALL=C
cd "$(dirname "$0")"
check=0
[ "${1:-}" = "--check" ] && check=1
pot=po/kcm_applekeyboard.pot
if [ $check = 1 ]; then
  tmp=$(mktemp -d "${TMPDIR:-/tmp}/akm-kcm-po.XXXXXX")
  trap 'rm -rf "$tmp"' EXIT
  pot=$tmp/kcm_applekeyboard.pot
fi
xgettext --from-code=UTF-8 --language=JavaScript --add-comments=i18n: \
  -ki18n:1 -ki18nc:1c,2 -ki18np:1,2 -ki18ncp:1c,2,3 \
  --package-name=kcm_applekeyboard \
  --msgid-bugs-address=https://gitea.pika.agenceapi.fr/adminapi/apple-kb-monitor/issues \
  -o "$pot" ui/*.qml
rc=0
for po in po/*/kcm_applekeyboard.po; do
  if [ $check = 1 ]; then
    merged=$tmp/$(basename "$(dirname "$po")").po
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
