#!/bin/bash
# Builds ToggleMail for Apple's stores on the Mac and uploads it to App Store Connect.
#   bash scripts/apple-build.sh mac   Mac App Store: universal .app (sandboxed), signed .pkg, upload
#   bash scripts/apple-build.sh ios   iPhone + iPad (+ the watch app, once gen/apple has it): archive, upload
# Run it in Terminal on the Mac (signing needs the unlocked login keychain, which SSH sessions don't have).
# Needs: Xcode, rustup targets (aarch64/x86_64-apple-darwin, aarch64-apple-ios), tauri-cli, the App Store Connect
# API key in ~/.appstoreconnect (ASC_KEY_ID / ASC_ISSUER below), and for `mac`:
#   - certificates "Apple Distribution" and "3rd Party Mac Developer Installer" (Xcode > Settings > Accounts),
#   - ../ToggleMail_Mac_App_Store.provisionprofile next to the repo (Mac App Store profile for com.toggleminds.togglemail).
set -euo pipefail
cd "$(dirname "$0")/.."
TEAM=${TEAM:-RAKS7AXSK2}
ASC_KEY_ID=${ASC_KEY_ID:-D38P4ZFFBF}
ASC_ISSUER=${ASC_ISSUER:-$(sed -n 's/^ASC_ISSUER_ID *= *//p' ~/.appstoreconnect/config 2>/dev/null)}
[ -n "$ASC_ISSUER" ] || { echo "No ASC_ISSUER_ID in ~/.appstoreconnect/config"; exit 1; }
export API_PRIVATE_KEYS_DIR=~/.appstoreconnect/private_keys
export LANG=en_US.UTF-8
[ -f ~/.cargo/env ] && . ~/.cargo/env
export PATH=/opt/homebrew/bin:$PATH
upload() { # $1 = macos|ios, $2 = file
  xcrun altool --upload-app --type "$1" --file "$2" --apiKey "$ASC_KEY_ID" --apiIssuer "$ASC_ISSUER"
}

case "${1:-}" in
  mac)
    [ -f ../ToggleMail_Mac_App_Store.provisionprofile ] || { echo "Missing ../ToggleMail_Mac_App_Store.provisionprofile"; exit 1; }
    rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
    APPLE_SIGNING_IDENTITY="Apple Distribution" \
      cargo tauri build --bundles app --target universal-apple-darwin --config src-tauri/tauri.appstore.conf.json
    app=src-tauri/target/universal-apple-darwin/release/bundle/macos/ToggleMail.app
    pkg=src-tauri/target/universal-apple-darwin/release/bundle/ToggleMail.pkg
    codesign --verify --deep --strict "$app"
    codesign -d --entitlements - "$app" | grep -q app-sandbox || { echo "Not sandboxed: check tauri.appstore.conf.json"; exit 1; }
    xcrun productbuild --sign "3rd Party Mac Developer Installer" --component "$app" /Applications "$pkg"
    upload macos "$pkg" ;;
  ios)
    rustup target add aarch64-apple-ios >/dev/null
    [ -d src-tauri/gen/apple ] || cargo tauri ios init
    # Apple Watch: apple/watch is a target in gen/apple/project.yml, embedded in the iPhone app.
    (cd src-tauri/gen/apple && xcodegen generate)
    # Lets Xcode make the App Store profiles for the iPhone and watch apps itself.
    APPLE_API_KEY=$ASC_KEY_ID APPLE_API_ISSUER=$ASC_ISSUER APPLE_API_KEY_PATH=$API_PRIVATE_KEYS_DIR/AuthKey_$ASC_KEY_ID.p8 \
      cargo tauri ios build --export-method app-store-connect
    ipa=$(find src-tauri/gen/apple/build -name '*.ipa' -newer src-tauri/gen/apple/project.yml | head -1)
    [ -n "$ipa" ] || { echo "No .ipa was made"; exit 1; }
    upload ios "$ipa" ;;
  *)
    echo "Usage: bash scripts/apple-build.sh mac|ios"; exit 1 ;;
esac
