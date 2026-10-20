#!/bin/bash
set -euo pipefail

# Credentials are installed by the release workflow in a temporary keychain.
: "${RELEASE_VERSION:?}"
: "${BUILD_NUMBER:?}"
: "${SPARKLE_PRIVATE_KEY_FILE:?}"
: "${NOTARY_KEYCHAIN:?}"
[[ "$RELEASE_VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo 'Expected X.Y.Z version' >&2; exit 1; }
[[ "$BUILD_NUMBER" =~ ^[1-9][0-9]*$ ]] || { echo 'Expected positive build number' >&2; exit 1; }

boltffi pack apple --release
CONFIGURATION=release bash apps/tunic-native/build-bundle.sh
app="$PWD/target/native/Tunic.app"
output="$PWD/target/release-native"
rm -rf "$output"
mkdir -p "$output"
ditto -c -k --sequesterRsrc --keepParent "$app" "$output/notarize.zip"
xcrun notarytool submit "$output/notarize.zip" --keychain "$NOTARY_KEYCHAIN" \
    --keychain-profile tunic-notary --wait
xcrun stapler staple "$app"
xcrun stapler validate "$app"
codesign --verify --deep --strict "$app"
spctl --assess --type execute --verbose "$app"
rm "$output/notarize.zip"
# Sign exactly the bytes users download, after stapling.
ditto -c -k --sequesterRsrc --keepParent "$app" "$output/Tunic.zip"
tools=apps/tunic-native/.build/artifacts/sparkle/Sparkle/bin
"$tools/generate_appcast" --ed-key-file "$SPARKLE_PRIVATE_KEY_FILE" \
    --download-url-prefix "https://github.com/bcinman/tunic/releases/download/v$RELEASE_VERSION/" \
    --maximum-deltas 0 "$output"
# Verify the signing secret matches the public key embedded in this release.
signature=$(/usr/bin/xmllint --xpath 'string(//enclosure/@*[local-name()="edSignature"])' "$output/appcast.xml")
swift - "$output/Tunic.zip" "$signature" "$SPARKLE_PUBLIC_KEY" <<'SWIFT'
import CryptoKit
import Foundation
let args = CommandLine.arguments
let archive = try Data(contentsOf: URL(fileURLWithPath: args[1]))
guard let signature = Data(base64Encoded: args[2]), let publicKey = Data(base64Encoded: args[3]),
      try Curve25519.Signing.PublicKey(rawRepresentation: publicKey).isValidSignature(signature, for: archive)
else {
    FileHandle.standardError.write(Data("Update signature does not match the embedded Sparkle public key\n".utf8))
    exit(1)
}
SWIFT
