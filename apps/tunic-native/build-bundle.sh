#!/bin/bash
set -euo pipefail

# Run from the repository root through mise.
package=apps/tunic-native
configuration=${CONFIGURATION:-debug}
identity=${CODE_SIGN_IDENTITY:--}
if [[ "$configuration" == release ]]; then
    : "${RELEASE_VERSION:?Required for release bundles}"
    : "${BUILD_NUMBER:?Required for release bundles}"
    : "${SPARKLE_PUBLIC_KEY:?Required for release bundles}"
    [[ "$identity" == "Developer ID Application:"* ]] || { echo 'Release requires Developer ID signing' >&2; exit 1; }
fi
swift build -c "$configuration" --package-path "$package" --product Tunic
bin=$(swift build -c "$configuration" --package-path "$package" --show-bin-path)
app="$PWD/target/native/Tunic.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$app/Contents/Frameworks"
cp "$bin/Tunic" "$app/Contents/MacOS/Tunic"
cp "$package/Resources/Info.plist" "$app/Contents/Info.plist"
ditto "$bin/TunicNative_TunicUI.bundle" "$app/Contents/Resources/TunicNative_TunicUI.bundle"
ditto "$bin/Sparkle.framework" "$app/Contents/Frameworks/Sparkle.framework"
if [[ "$configuration" == release ]]; then
    plist="$app/Contents/Info.plist"
    plutil -replace CFBundleShortVersionString -string "$RELEASE_VERSION" "$plist"
    plutil -replace CFBundleVersion -string "$BUILD_NUMBER" "$plist"
    plutil -insert SUPublicEDKey -string "$SPARKLE_PUBLIC_KEY" "$plist"
    plutil -insert SUFeedURL -string 'https://github.com/bcinman/tunic/releases/latest/download/appcast.xml' "$plist"
fi
# Sign nested code inside-out, never with --deep.
framework="$app/Contents/Frameworks/Sparkle.framework"
for nested in XPCServices/Installer.xpc XPCServices/Downloader.xpc Autoupdate Updater.app; do
    codesign --force --sign "$identity" --options runtime --timestamp "$framework/Versions/B/$nested"
done
codesign --force --sign "$identity" --options runtime --timestamp "$framework"
entitlements=$(mktemp)
trap 'rm -f "$entitlements"' EXIT
cp "$package/Resources/Tunic.entitlements" "$entitlements"
if [[ "$identity" == - ]]; then
    # Ad-hoc code has no Team ID for hardened library validation.
    plutil -insert 'com\.apple\.security\.cs\.disable-library-validation' -bool YES "$entitlements"
fi
codesign --force --sign "$identity" --options runtime --timestamp \
    --entitlements "$entitlements" "$app"
codesign --verify --deep --strict "$app"
printf 'Built %s\n' "$app"
