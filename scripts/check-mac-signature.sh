#!/bin/sh
# Fails unless every packaged Hormuz.app in <dir> is Developer ID signed (bundled tools included), notarized with
# the ticket stapled, and accepted by Gatekeeper, i.e. a downloaded copy opens without "can't be opened" or "damaged".
# With a second folder, every .dmg in it must pass the same signature, notarization and Gatekeeper checks.
#   scripts/check-mac-signature.sh target/universal-apple-darwin/release/bundle/macos dist
set -eu
dist=${1:?usage: check-mac-signature.sh <folder with Hormuz.app> [folder with .dmg files]}
dmgs=${2:-}
found=0
for app in "$dist"/Hormuz.app; do
  [ -d "$app" ] || continue
  found=1
  echo "== $app"
  codesign --verify --deep --strict --verbose=2 "$app"
  codesign -dvv "$app" 2>&1 | grep -q '^Authority=Developer ID Application:' || { echo "not signed with a Developer ID Application certificate"; exit 1; }
  # every bundled executable and library needs the hardened runtime for notarization
  find "$app/Contents/Resources" -type f -perm -u+x -o -type f \( -name '*.dylib' -o -name '*.so' -o -name '*.node' \) | while read -r f; do
    kind=$(file -b "$f")
    case $kind in *Mach-O*) ;; *) continue ;; esac
    info=$(codesign -dvv "$f" 2>&1) || { echo "unsigned: $f"; exit 1; }
    echo "$info" | grep -q 'Authority=Developer ID Application:' || { echo "not Developer ID signed: $f"; exit 1; }
    case $kind in *executable*) echo "$info" | grep -q 'flags=.*runtime' ;; esac || { echo "no hardened runtime: $f"; exit 1; }
  done
  xcrun stapler validate "$app"
  spctl --assess --type execute --verbose=2 "$app" 2>&1 | tee /dev/stderr | grep -q 'source=Notarized Developer ID' || { echo "Gatekeeper does not accept it as notarized"; exit 1; }
done
[ "$found" = 1 ] || { echo "no Hormuz.app under $dist"; exit 1; }

[ -n "$dmgs" ] || exit 0
found=0
for dmg in "$dmgs"/*.dmg; do
  [ -f "$dmg" ] || continue
  found=1
  echo "== $dmg"
  codesign --verify --strict --verbose=2 "$dmg"
  codesign -dvv "$dmg" 2>&1 | grep -q '^Authority=Developer ID Application:' || { echo "not signed with a Developer ID Application certificate"; exit 1; }
  xcrun stapler validate "$dmg"
  spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg" 2>&1 | tee /dev/stderr | grep -q 'source=Notarized Developer ID' || { echo "Gatekeeper does not accept the .dmg as notarized"; exit 1; }
done
[ "$found" = 1 ] || { echo "no .dmg under $dmgs"; exit 1; }
