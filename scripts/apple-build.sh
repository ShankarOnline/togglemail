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
# Every upload needs a higher build number than the last: the time of the build, e.g. 2611161430.
BUILD=${BUILD:-$(date +%y%m%d%H%M)}
echo "Build number $BUILD"
upload() { # $1 = macos|ios, $2 = file
  xcrun altool --upload-app --type "$1" --file "$2" --apiKey "$ASC_KEY_ID" --apiIssuer "$ASC_ISSUER"
}

case "${1:-}" in
  mac)
    [ -f ../ToggleMail_Mac_App_Store.provisionprofile ] || { echo "Missing ../ToggleMail_Mac_App_Store.provisionprofile"; exit 1; }
    rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
    APPLE_SIGNING_IDENTITY="Apple Distribution" \
      cargo tauri build --bundles app --target universal-apple-darwin --config src-tauri/tauri.appstore.conf.json \
      --config "{\"bundle\":{\"macOS\":{\"bundleVersion\":\"$BUILD\"}}}"
    app=src-tauri/target/universal-apple-darwin/release/bundle/macos/ToggleMail.app
    pkg=src-tauri/target/universal-apple-darwin/release/bundle/ToggleMail.pkg
    codesign --verify --deep --strict "$app"
    for e in app-sandbox application-identifier; do
      codesign -d --entitlements - "$app" 2>/dev/null | grep -q "$e" || { echo "Missing entitlement $e: check tauri.appstore.conf.json"; exit 1; }
    done
    [ "$(defaults read "$PWD/$app/Contents/Info" CFBundleVersion)" = "$BUILD" ] || { echo "Build number not set"; exit 1; }
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
  <key>signingStyle</key><string>manual</string>
  <key>signingCertificate</key><string>Apple Distribution</string>
  <key>teamID</key><string>$TEAM</string>
  <key>provisioningProfiles</key><dict>
    <key>com.toggleminds.togglemail</key><string>ToggleMail iOS App Store</string>
    <key>com.toggleminds.togglemail.watchkitapp</key><string>ToggleMail Watch App Store</string>
  </dict>
  <key>uploadSymbols</key><true/>
</dict></plist>
EOF
    # Signed with the Apple Distribution certificate in the login keychain and the two App Store profiles
    # (made with the App Store Connect API; the API key can't use Xcode's cloud signing).
    xcodebuild archive -project src-tauri/gen/apple/togglemail.xcodeproj -scheme togglemail_iOS -configuration release \
      -destination generic/platform=iOS -archivePath "$out/ToggleMail.xcarchive" TOGGLEMAIL_RUST_PREBUILT=1 CURRENT_PROJECT_VERSION="$BUILD" \
      CODE_SIGNING_ALLOWED=NO -quiet
    xcodebuild -exportArchive -archivePath "$out/ToggleMail.xcarchive" -exportOptionsPlist "$out/ExportOptions.plist" \
      -exportPath "$out" -quiet
    upload ios "$out/ToggleMail.ipa" ;;
  *)
    echo "Usage: bash scripts/apple-build.sh mac|ios"; exit 1 ;;
esac
