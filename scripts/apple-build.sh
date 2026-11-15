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
    # The Rust core is built here, as `cargo tauri ios build` would, and Xcode is run directly: tauri passes
    # -sdk iphoneos to Xcode, which would build the embedded watch app for the iPhone too.
    # A static library only: the watch functions it calls are in WatchBridge.swift, linked in by Xcode.
    (cd src-tauri && cargo rustc --release --lib --crate-type staticlib --target aarch64-apple-ios --features tauri/custom-protocol)
    mkdir -p src-tauri/gen/apple/Externals/arm64/release
    cp src-tauri/target/aarch64-apple-ios/release/libtogglemail_lib.a src-tauri/gen/apple/Externals/arm64/release/libapp.a
    out=$PWD/build/apple-ios
    rm -rf "$out" && mkdir -p "$out"
    cat > "$out/ExportOptions.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>method</key><string>app-store-connect</string>
  <key>signingStyle</key><string>automatic</string>
  <key>teamID</key><string>$TEAM</string>
  <key>uploadSymbols</key><true/>
</dict></plist>
EOF
    # Lets Xcode make the App Store profiles for the iPhone and watch apps itself.
    auth=(-allowProvisioningUpdates -authenticationKeyID "$ASC_KEY_ID" -authenticationKeyIssuerID "$ASC_ISSUER"
          -authenticationKeyPath "$API_PRIVATE_KEYS_DIR/AuthKey_$ASC_KEY_ID.p8")
    xcodebuild archive -project src-tauri/gen/apple/togglemail.xcodeproj -scheme togglemail_iOS -configuration release \
      -destination generic/platform=iOS -archivePath "$out/ToggleMail.xcarchive" TOGGLEMAIL_RUST_PREBUILT=1 "${auth[@]}" -quiet
    xcodebuild -exportArchive -archivePath "$out/ToggleMail.xcarchive" -exportOptionsPlist "$out/ExportOptions.plist" \
      -exportPath "$out" "${auth[@]}" -quiet
    upload ios "$out/ToggleMail.ipa" ;;
  *)
    echo "Usage: bash scripts/apple-build.sh mac|ios"; exit 1 ;;
esac
