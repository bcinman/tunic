#!/bin/bash
set -euo pipefail

die() { printf '%s\n' "$*" >&2; exit 1; }
if [[ $# == 1 && "$1" == --help ]]; then
    echo 'Usage: mise run release X.Y.Z [--upload]'
    echo 'Build locally using Keychain, then ask before uploading a draft.'
    echo '--upload reuses verified artifacts from a previous successful build.'
    exit 0
fi
[[ $# -ge 1 && $# -le 2 ]] || die 'Usage: mise run release X.Y.Z [--upload]'
version=$1
[[ "$version" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]] || die 'Expected a version like 0.1.0'
[[ $# == 1 || "$2" == --upload ]] || die 'Only --upload is supported after the version'
root=$(git rev-parse --show-toplevel)
[[ "$PWD" == "$root" ]] || die 'Run this task from the repository root'
repo=bcinman/tunic
tag="v$version"
output="$root/target/releases/$version"

gh auth status >/dev/null
[[ $(gh repo view "$(git remote get-url origin)" --json nameWithOwner --jq .nameWithOwner) == "$repo" ]] ||
    die "Expected origin to be $repo"
if [[ $# == 2 ]]; then
    [[ -f "$output/SHA256SUMS" ]] || die 'No completed release artifacts to upload'
    source_commit=$(cat "$output/source-commit")
    [[ "$source_commit" =~ ^[0-9a-f]{40}$ ]] || die 'Invalid saved source commit'
else
    # jj's normal working copy is an empty commit on top of the recorded source.
    [[ $(jj log -r @ --no-graph -T empty) == true ]] || die 'Commit your changes with jj first'
    source_commit=$(jj log -r 'parents(@)' --no-graph -T 'commit_id ++ "\n"')
    [[ "$source_commit" =~ ^[0-9a-f]{40}$ ]] || die 'Expected one working-copy parent'
    [[ ! -e "$output" ]] || die "Output already exists: $output. Use --upload for completed artifacts."
fi

# Only release source already reviewed and pushed to main. Fetching does not
# move the working copy or publish any local changes.
git fetch origin main
git merge-base --is-ancestor "$source_commit" FETCH_HEAD || die 'Release source must already be on origin/main'
releases=$(gh api --paginate "repos/$repo/releases" --jq '.[].tag_name')
RELEASE_TAGS="$releases" python -I - "$version" <<'PY'
import os, re, sys
current = tuple(map(int, sys.argv[1].split('.')))
for tag in os.environ['RELEASE_TAGS'].splitlines():
    if re.fullmatch(r'v[0-9]+\.[0-9]+\.[0-9]+', tag):
        if tuple(map(int, tag[1:].split('.'))) >= current:
            raise SystemExit(f'Choose a version newer than existing release {tag} (including drafts)')
PY
remote_tag=$(git ls-remote --tags origin "refs/tags/$tag" "refs/tags/$tag^{}")
tag_commit=$(printf '%s\n' "$remote_tag" | awk '/\^\{\}$/ { peeled=$1 } NF { raw=$1 } END { print peeled ? peeled : raw }')
[[ -z "$tag_commit" || "$tag_commit" == "$source_commit" ]] || die 'Remote tag points at a different commit'

if [[ $# == 1 ]]; then
    identities=$(security find-identity -v -p codesigning | sed -n 's/.*"\(Developer ID Application:.*\)".*/\1/p')
    if [[ -z "${CODE_SIGN_IDENTITY:-}" ]]; then
        [[ -n "$identities" && $(printf '%s\n' "$identities" | wc -l | tr -d ' ') == 1 ]] ||
            die 'Set CODE_SIGN_IDENTITY to one valid Developer ID Application identity from your Keychain'
        export CODE_SIGN_IDENTITY="$identities"
    fi
    grep -Fxq "$CODE_SIGN_IDENTITY" <<< "$identities" || die 'Signing identity is not available in Keychain'
    xcrun notarytool history --keychain-profile tunic-notary >/dev/null
    mise run check
    tools="$root/apps/tunic-native/.build/artifacts/sparkle/Sparkle/bin"
    export SPARKLE_PUBLIC_KEY
    SPARKLE_PUBLIC_KEY=$("$tools/generate_keys" --account tunic -p)
    [[ -n "$SPARKLE_PUBLIC_KEY" ]] || die 'Set up the Sparkle tunic Keychain account first'
    boltffi pack apple --release
    CONFIGURATION=release RELEASE_VERSION="$version" BUILD_NUMBER="$version" \
        bash apps/tunic-native/build-bundle.sh
    mkdir -p "$output"
    app="$root/target/native/Tunic.app"
    ditto -c -k --sequesterRsrc --keepParent "$app" "$output/notarize.zip"
    xcrun notarytool submit "$output/notarize.zip" --keychain-profile tunic-notary --wait
    xcrun stapler staple "$app"
    xcrun stapler validate "$app"
    codesign --verify --deep --strict "$app"
    spctl --assess --type execute --verbose "$app"
    rm "$output/notarize.zip"
    ditto -c -k --sequesterRsrc --keepParent "$app" "$output/Tunic.zip"
    "$tools/generate_appcast" --account tunic --maximum-deltas 0 \
        --download-url-prefix "https://github.com/$repo/releases/download/$tag/" "$output"
    signature=$(xmllint --xpath 'string(//enclosure/@*[local-name()="edSignature"])' "$output/appcast.xml")
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
    [[ $(jj log -r @ --no-graph -T empty) == true &&
       $(jj log -r 'parents(@)' --no-graph -T 'commit_id ++ "\n"') == "$source_commit" ]] ||
        die 'Source changed during the build; artifacts will not be marked ready'
    printf '%s\n' "$source_commit" > "$output/source-commit"
    # Relative names make verification work when retrying only the upload.
    (cd "$output" && shasum -a 256 Tunic.zip appcast.xml source-commit > SHA256SUMS.tmp &&
        mv SHA256SUMS.tmp SHA256SUMS)
fi

(cd "$output" && shasum -a 256 -c SHA256SUMS)
python -I - "$output" "$version" "$repo" <<'PY'
import pathlib, plistlib, sys, xml.etree.ElementTree as ET, zipfile
directory, version, repo = pathlib.Path(sys.argv[1]), sys.argv[2], sys.argv[3]
with zipfile.ZipFile(directory / 'Tunic.zip') as archive:
    info = plistlib.loads(archive.read('Tunic.app/Contents/Info.plist'))
assert info['CFBundleVersion'] == info['CFBundleShortVersionString'] == version
item = ET.parse(directory / 'appcast.xml').find('./channel/item')
ns = '{http://www.andymatuschak.org/xml-namespaces/sparkle}'
assert item.find(ns + 'version').text == version
enclosure = item.find('enclosure')
assert enclosure.get('url') == f'https://github.com/{repo}/releases/download/v{version}/Tunic.zip'
assert int(enclosure.get('length')) == (directory / 'Tunic.zip').stat().st_size
PY
printf '\nPrepared %s from %s\nArtifacts: %s\n' "$tag" "$source_commit" "$output"
printf 'Create/push tag %s and upload a DRAFT release to %s? [y/N] ' "$tag" "$repo"
if ! read -r answer || [[ "$answer" != y ]]; then
    echo "Not uploaded. Resume with: mise run release $version --upload"
    exit 0
fi
# Push only this tag, never move main or rewrite an existing tag.
if [[ -z "$tag_commit" ]]; then
    git push origin "$source_commit:refs/tags/$tag"
fi
gh release create "$tag" --repo "$repo" --verify-tag --draft --title "Tunic $version" \
    --generate-notes "$output/Tunic.zip" "$output/appcast.xml"
echo 'Draft created. Download and test it before publishing manually as the latest release.'
