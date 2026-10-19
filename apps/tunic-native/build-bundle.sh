#!/bin/bash
set -euo pipefail

# Run from the repository root through mise. This is a local development bundle, not a release.
package=apps/tunic-native
swift build --package-path "$package" --product Tunic
bin=$(swift build --package-path "$package" --show-bin-path)
app="$PWD/target/native/Tunic.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$bin/Tunic" "$app/Contents/MacOS/Tunic"
cp "$package/Resources/Info.plist" "$app/Contents/Info.plist"
ditto "$bin/TunicNative_TunicUI.bundle" "$app/Contents/Resources/TunicNative_TunicUI.bundle"
codesign --force --sign "${CODE_SIGN_IDENTITY:--}" --options runtime \
    --entitlements "$package/Resources/Tunic.entitlements" "$app"
codesign --verify --strict "$app"
printf 'Built %s\n' "$app"
