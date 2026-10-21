# Releasing Tunic

Releases are prepared on your Mac, using signing credentials in Keychain.
GitHub Releases hosts the downloads; Sparkle checks for updates inside Tunic.
GitHub Actions runs PR/main checks only. Merging code does not publish an app.

```text
Your Mac: checks → build → sign → notarize → staple → ZIP + appcast
                                                           │
                                                  Confirm upload
                                                           ↓
                                                 Draft GitHub Release
                                                           │
                                                  Test, then publish
```

The entry point is `mise run release VERSION`, implemented in
[`script/release.sh`](../script/release.sh). Run commands from the repository root.

## One-time setup

You need Xcode, the tools installed by `mise install`, `jj`, and `gh` on your Mac.
Use this colocated jj/Git checkout with `origin` pointing to `bcinman/tunic`.

### GitHub access

```sh
gh auth login
```

Your account needs permission to push tags and create releases in the repository.
No GitHub release environment or signing secrets are used. Credentials left from
the former Actions release workflow can be removed separately.

### Apple signing identity

With an active paid Apple Developer Program membership, open **Xcode → Settings →
Accounts → your team → Manage Certificates** and create a **Developer ID
Application** identity. This is not an Apple Development, Apple Distribution, or
Developer ID Installer certificate.

The certificate and its private key must be available in your login Keychain:

```sh
security find-identity -v -p codesigning
```

The script selects the identity automatically if exactly one valid Developer ID
Application identity exists. If there are several, select one explicitly:

```sh
export CODE_SIGN_IDENTITY='Developer ID Application: Your Name (TEAMID)'
```

Use the complete identity name from the command output. No `.p12` export or
certificate password is required for routine local releases.

### Apple notarization credentials

Create an app-specific password at [account.apple.com](https://account.apple.com/)
under **Sign-In and Security → App-Specific Passwords**. This requires two-factor
authentication; do not use your normal Apple Account password.

Store the credentials in Keychain using the exact profile name below:

```sh
xcrun notarytool store-credentials tunic-notary
```

Follow the prompts for your Apple Account email, developer team ID, and
app-specific password. Do not put credentials in source files or chat.

### Sparkle signing key

Build once to download Sparkle and its tools, then set up the `tunic` Keychain
account:

```sh
mise run build-native-bundle
tools=apps/tunic-native/.build/artifacts/sparkle/Sparkle/bin
"$tools/generate_keys" --account tunic
```

This reuses an existing key for `tunic`, or creates one if absent. Allow Keychain
access if prompted. The release script reads the public key and signs updates
through Keychain without exporting the private key.

Keep secure backups of your Apple and Sparkle signing keys. Keep the same Sparkle
key and app bundle identifier across releases; installed apps trust that public
key. Never casually regenerate it to resolve a signing problem.

## Release a new version

### 1. Prepare the source revision

Record your changes with jj, push your bookmark, and merge the PR into GitHub
`main`. Then fetch and start an empty working-copy commit above main:

```sh
jj git fetch
jj new main@origin
```

The script builds the parent of that empty working-copy commit. The parent must
already be on GitHub `main` (an ancestor of its fetched tip is also accepted).
It does not commit changes, move main, or merge a PR for you.

### 2. Build and package

Choose a new three-part version, without the `v` prefix or leading zeros:

```sh
mise run release 0.1.0
```

Both the displayed version and the internal `CFBundleVersion` become `0.1.0`.
The eventual GitHub tag is `v0.1.0`. Versions are compared numerically, so
`0.10.0` is newer than `0.2.0`.

The script checks credentials and version availability, then:

1. Runs `mise run check`.
2. Builds Rust and Swift in release mode and assembles `Tunic.app`.
3. Signs the app and Sparkle helpers using your Developer ID identity.
4. Submits the app to Apple for notarization and waits for the result.
5. Staples Apple's ticket to the app and checks its signature and Gatekeeper assessment.
6. Creates the final ZIP, generates Sparkle's signed update entry, and verifies
   the archive signature against the public key embedded during the build.
7. Records the source commit and artifact checksums, then validates version,
   download URL, and archive size metadata.

Keep your Mac online and respond to Keychain prompts. Do not edit the source or
run another bundle build during this process. The script checks that the source
is unchanged before marking artifacts ready.

Completed artifacts are kept locally:

```text
target/releases/0.1.0/
├── Tunic.zip
├── appcast.xml
├── source-commit
└── SHA256SUMS
```

`appcast.xml` is the small document Sparkle reads to discover the version,
requirements, download URL, and update signature. Only the ZIP and appcast are
uploaded; the source record and checksums support local upload retries.

### 3. Confirm draft upload

The script prints the source commit and artifact location, then asks:

```text
Create/push tag v0.1.0 and upload a DRAFT release to bcinman/tunic? [y/N]
```

Enter exactly `y` to push the tag and create the draft. It never publishes the
release or rewrites an existing tag. A matching existing tag can be reused.

Any other answer, including end-of-input, leaves artifacts local. Upload them
later without rebuilding, signing, or notarizing again:

```sh
mise run release 0.1.0 --upload
```

This rechecks the saved artifacts, source eligibility, and version availability,
then asks for confirmation again. Unlike building, `--upload` does not require an
empty jj working copy: it uses the saved source commit, not your current code.

### 4. Test the draft

Open the draft URL printed by the script. Download `Tunic.zip`, unzip it, and
install Tunic in Applications. Test:

- Launch and macOS permission prompts.
- Audio processing, bypass, and saved profiles.
- Now Playing and Open at Login.
- The update controls in the gear menu.

Review and edit the generated release notes. A draft is not public, and Sparkle
cannot download its assets anonymously.

**Before the first public launch, test a genuine older → newer signed/notarized
Sparkle update using a temporary test feed.** Preparing a draft does not prove
that installation, relaunch, or permission continuity works. This end-to-end
validation remains outstanding; the script does not automate it.

### 5. Publish manually

Publish the draft in GitHub and mark it as the **latest release**. Publish in
version order. This exposes both the download and the update feed:

```text
https://github.com/bcinman/tunic/releases/latest/download/appcast.xml
```

The feed points to that version's ZIP. Installed copies of Tunic discover it on
their next automatic or manual update check; publishing does not immediately
push an update to every Mac.

## Refusals and recovery

| Situation | What happens / what to do |
| --- | --- |
| jj `@` contains changes | Stops with `Commit your changes with jj first`. A description alone does not make a change empty. Record/merge your intended changes, then use an empty working copy above the release source. Nothing is discarded. |
| Source is not on GitHub main | Stops before building. Merge it first, then fetch. |
| Version already exists or is lower | Stops before building/uploading. It checks all matching `vX.Y.Z` GitHub releases, including drafts, not just the one marked latest. Choose a higher version. |
| Remote tag points to another commit | Stops; it never overwrites that tag. |
| Local version directory already exists | Refuses to overwrite it. Use `--upload` if preparation completed. For an incomplete build, inspect the files and move the directory aside before rebuilding. |
| Missing identity or Keychain profile | Stops. Complete the one-time setup; do not export credentials into the script. |
| Source changes during the build | Refuses to mark artifacts ready. Restore a stable source revision and rebuild. |
| Saved artifacts changed | Checksum or metadata validation fails; nothing uploads. Do not edit the ZIP or feed after preparation. |
| Tag push succeeded but draft creation failed | Retry with `--upload`; the matching tag can be reused. |
| Draft exists but asset upload failed | Inspect that draft and complete its assets manually. The script refuses to overwrite any existing release, including a draft. |

If a published version is bad, release a **higher version** containing the fix
or revert. Never replace a published ZIP or downgrade the feed.

The current release targets Apple Silicon on macOS 26+. Before changing the
minimum macOS version or supported architectures, extend the feed strategy to
retain updates for users on older supported systems.
