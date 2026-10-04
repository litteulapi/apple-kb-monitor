#!/bin/sh
# Catalogs of akmctl, the daemon and akm-helper: complete, valid, and
# loaded at run time (French with LANG=fr_FR.UTF-8, English with LANG=C).
set -eu
top=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/akm-po.XXXXXX")
trap 'rm -rf "$work"' EXIT

for po in "$top"/po/*.po; do
    lang=$(basename "$po" .po)
    stats=$(LC_ALL=C msgfmt --check --statistics -o "$work/$lang.mo" "$po" 2>&1)
    case $stats in
        *fuzzy*|*untranslated*) echo "FAIL: $lang: $stats"; exit 1 ;;
    esac
    mkdir -p "$work/locale/$lang/LC_MESSAGES"
    mv "$work/$lang.mo" "$work/locale/$lang/LC_MESSAGES/apple-kb-monitor.mo"
done

# translators report to the public tracker, the one of every other catalog
if grep -lE "Report-Msgid-Bugs-To: [^\\\"]*gitea|bugs-address=[^ ]*gitea" "$top"/po/*.po "$top"/po/*.pot "$top"/scripts/update-po.sh; then
    echo "FAIL: Report-Msgid-Bugs-To points to a Gitea tracker"; exit 1
fi

# xgettext -L Rust skips a path-qualified macro call: crate::tr!("x") never reaches the catalog
if grep -rnE '::(tr|trn|N_)!' "$top"/apihub-app --include='*.rs' | grep -v '/target/'; then
    echo "FAIL: path-qualified tr!/trn!/N_! above: import the macro and call it unqualified"; exit 1
fi

akmctl=${CARGO_TARGET_DIR:-$top/apihub-app/target}/debug/akmctl
run=""
if [ -x "$akmctl" ] && locale -a 2>/dev/null | grep -qix "fr_FR.utf8"; then
    run=", akmctl fr/C checked"
    fr=$(AKM_LOCALEDIR="$work/locale" LANG=fr_FR.UTF-8 LC_ALL='' LANGUAGE='' "$akmctl" --help | head -1)
    en=$(AKM_LOCALEDIR="$work/locale" LANG=C LC_ALL='' LANGUAGE='' "$akmctl" --help | head -1)
    [ "$fr" != "$en" ] || { echo "FAIL: akmctl --help is not translated: $fr"; exit 1; }
    case $en in Control*) ;; *) echo "FAIL: akmctl --help under LANG=C: $en"; exit 1 ;; esac
fi
echo "po: $(set -- "$top"/po/*.po; echo $#) catalog(s) complete$run"
