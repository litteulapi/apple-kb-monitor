#!/bin/sh
# No trademark of a third-party game universe in what the project ships or
# documents. The art direction keeps its look (green phosphor, CRT,
# VT323); the names on screen are the application's own (APIHUB).
#
# 1. Brand names of that universe: nowhere in the tracked tree.
# 2. "Pip-Boy" as a name shown to the user: never in a translatable string
#    (.po msgid/msgstr), a QML `text:`/i18n() literal or a Rust tr() literal.
#    Internal file and document names stay.
# A line marked `trademark-check: list` (a test's own list of forbidden
# names) is not a use.
# Exit 0 = clean, 1 = a forbidden name was found (listed).
set -u
cd "$(dirname "$0")/.." || exit 2
self=tests/check-trademarks.sh
brands='robco|vault-?tec|bethesda|fallout|nuka-?cola|zenimax|termlink'
bad=0

hits=$(git ls-files -z | grep -zv "^$self\$" | xargs -0 grep -IinE "$brands" -- 2>/dev/null | grep -v 'trademark-check: list')
if [ -n "$hits" ]; then
  echo "third-party brand names in the tree:"
  echo "$hits" | head -40
  bad=1
fi

shown=$(
  git ls-files -z -- '*.po' '*.pot' | xargs -0 -r grep -inE '^(msgid|msgstr|")' -- 2>/dev/null | grep -iE 'pip.?boy'
  git ls-files -z -- '*.qml' | xargs -0 -r grep -inE '(text:|i18nc?\().*"[^"]*pip.?boy' -- 2>/dev/null
  git ls-files -z -- '*.rs' | xargs -0 -r grep -inE '\btr\("[^"]*pip.?boy' -- 2>/dev/null
)
if [ -n "$shown" ]; then
  echo "\"Pip-Boy\" shown to the user as a name:"
  echo "$shown" | head -40
  bad=1
fi

[ $bad = 0 ] && echo "no third-party trademark shown or documented"
exit $bad
