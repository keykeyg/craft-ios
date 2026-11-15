#!/bin/bash
# Build one app for iOS and (when CRAFT_KEY is set) sign it and upload it to TestFlight.
# Runs on a GitHub macOS runner.  Usage: build_app.sh <app id>   (ids: apps.json)
set -euo pipefail
ID="$1"
REPO="$(cd "$(dirname "$0")/.." && pwd)"
W="${RUNNER_TEMP:-/tmp}/kg-$ID"
rm -rf "$W" && mkdir -p "$W"
cfg() { python3 -c "import json,sys;a=[x for x in json.load(open('$REPO/apps.json')) if x['id']=='$ID'][0];print(a.get('$1',''))"; }
UP="$(cfg upstream)"; NAME="$(cfg name)"; REF="$(cfg ref)"
PRODUCT="KG$(python3 -c "print('$ID'.capitalize())")"
BUNDLE="com.keyangodarzi.kg.$ID"
BUILD="${GITHUB_RUN_NUMBER:-1}.${GITHUB_RUN_ATTEMPT:-1}"

step() { echo "::group::$*"; }
end() { echo "::endgroup::"; }

# --- upstream source -----------------------------------------------------------------------------
step "Fetch storytold/$UP ${REF:-(latest release)}"
if [ -z "$REF" ]; then
  REF="$(git ls-remote --tags --refs "https://github.com/storytold/$UP" 'v*' | sed 's#.*refs/tags/##' | sort -V | tail -1)"
fi
git clone --quiet --depth 1 ${REF:+--branch "$REF"} "https://github.com/storytold/$UP" "$W/src"
echo "ref=${REF:-main} commit=$(git -C "$W/src" rev-parse --short HEAD)" | tee "$W/upstream.txt"
python3 "$REPO/shim/apply.py" "$W/src" "$UP" "$(cfg brand)" "$NAME"
end

# --- Rust build ------------------------------------------------------------------------------------
step "cargo build $UP (aarch64-apple-ios, release)"
cd "$W/src"
export IPHONEOS_DEPLOYMENT_TARGET=17.0
export CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16
export CARGO_PROFILE_RELEASE_DEBUG=0 CARGO_PROFILE_RELEASE_STRIP=symbols
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$W/target}"
cargo build --release --target aarch64-apple-ios -p "$UP" 2>&1 | tee "$W/cargo.log"
BIN="$CARGO_TARGET_DIR/aarch64-apple-ios/release/$UP"
test -f "$BIN"; file "$BIN"; ls -la "$BIN"
# The SDK in the binary decides how iPhone Duo shows the app (27.1+: full inner display).
xcrun --sdk iphoneos --show-sdk-version; xcrun vtool -show-build "$BIN" | grep -E "minos|sdk" || true
VER="$(cargo metadata --no-deps --format-version 1 | python3 -c "import json,sys;print([p for p in json.load(sys.stdin)['packages'] if p['name']=='$UP'][0]['version'])")"
end

# --- bundle extras -----------------------------------------------------------------------------------
step "Bundle resources"
mkdir -p "$W/xc/Payload/Licenses"
for f in NOTICE LICENSE LICENSE-APACHE LICENSE-MIT ATTRIBUTION.md; do [ -f "$f" ] && cp "$f" "$W/xc/Payload/Licenses/"; done
cp "$REPO/NOTICE.ios.md" "$W/xc/Payload/Licenses/" 2>/dev/null || true
if [ "$UP" = pdfcraft ]; then
  if cargo xtask models; then mkdir -p "$W/xc/Payload/models" && cp assets/models/* "$W/xc/Payload/models/"; else echo "::warning::OCR models not fetched; OCR will be off"; fi
fi
end

# --- Xcode project -----------------------------------------------------------------------------------
step "Xcode project"
cp "$REPO"/harness/{project.yml,Info.plist,PrivacyInfo.xcprivacy,main.c} "$W/xc/"
ICONSET="$W/xc/Assets.xcassets/AppIcon.appiconset"
mkdir -p "$ICONSET"
cp "$REPO/icons/$ID.png" "$ICONSET/icon.png"
echo '{"info":{"author":"xcode","version":1}}' > "$W/xc/Assets.xcassets/Contents.json"
echo '{"images":[{"filename":"icon.png","idiom":"universal","platform":"ios","size":"1024x1024"}],"info":{"author":"xcode","version":1}}' > "$ICONSET/Contents.json"
command -v xcodegen >/dev/null || brew install xcodegen
(cd "$W/xc" && xcodegen generate --quiet)
end

# --- no signing key here: publish the unsigned app for signing elsewhere -------------------------------
if [ -z "${CRAFT_KEY:-}" ]; then
  step "Unsigned archive"
  cd "$W/xc"
  xcodebuild -project KGApp.xcodeproj -scheme KGApp -configuration Release -sdk iphoneos \
    -archivePath "$W/KG.xcarchive" archive CODE_SIGNING_ALLOWED=NO CODE_SIGNING_REQUIRED=NO CODE_SIGN_IDENTITY="" \
    KG_PRODUCT_NAME="$PRODUCT" KG_BUNDLE_ID="$BUNDLE" KG_DISPLAY_NAME="$NAME" \
    KG_VERSION="$VER" KG_BUILD="$BUILD" KG_TEAM="" KG_PROFILE="" \
    KG_RUST_BIN="$BIN" > "$W/xcodebuild.log" 2>&1 || { tail -60 "$W/xcodebuild.log"; exit 1; }
  APP="$W/KG.xcarchive/Products/Applications/$PRODUCT.app"
  test -x "$APP/$PRODUCT"; ls -la "$APP"
  ditto -c -k --keepParent "$APP" "$W/$ID.app.zip"
  printf '{"id":"%s","version":"%s","build":"%s","product":"%s","bundle":"%s","upstream":"%s"}\n' \
    "$ID" "$VER" "$BUILD" "$PRODUCT" "$BUNDLE" "$(cat "$W/upstream.txt")" > "$W/$ID.json"
  TAG="apps-${GITHUB_RUN_NUMBER:-0}"
  gh release view -R "$GITHUB_REPOSITORY" "$TAG" >/dev/null 2>&1 || gh release create -R "$GITHUB_REPOSITORY" "$TAG" --title "Unsigned iOS apps, run ${GITHUB_RUN_NUMBER:-0}" \
    --notes "Unsigned app bundles from the testflight workflow; signed and uploaded to TestFlight separately." --prerelease >/dev/null 2>&1 || true
  ok=; for i in 1 2 3 4 5; do gh release upload -R "$GITHUB_REPOSITORY" "$TAG" "$W/$ID.app.zip" "$W/$ID.json" --clobber && { ok=1; break; }; sleep $((i * 5)); done; [ -n "$ok" ]
  echo "published $ID.app.zip to release $TAG"
  end
  exit 0
fi

# --- signing material ----------------------------------------------------------------------------------
step "Signing"
pip3 install --quiet --break-system-packages cryptography pyjwt requests 2>/dev/null || pip3 install --quiet cryptography pyjwt requests
python3 "$REPO/scripts/vault.py" open "$REPO/secrets.enc" "$W/vault"
set -a; . "$W/vault/ids.env"; set +a
KC="$W/kg.keychain-db"; KCPW="$(uuidgen)"
security create-keychain -p "$KCPW" "$KC"
security set-keychain-settings -lut 21600 "$KC"
security unlock-keychain -p "$KCPW" "$KC"
curl -fsSL -o "$W/wwdr.cer" https://www.apple.com/certificateauthority/AppleWWDRCAG3.cer && security import "$W/wwdr.cer" -k "$KC" -A || true
security import "$W/vault/dist.p12" -k "$KC" -P "$(cat "$W/vault/p12_pass")" -A -T /usr/bin/codesign -T /usr/bin/security
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$KCPW" "$KC" >/dev/null
security list-keychains -d user -s "$KC" $(security list-keychains -d user | tr -d '"')
security find-identity -v -p codesigning "$KC"
PROFILE="$REPO/profiles/$ID.mobileprovision"
UUID="$(security cms -D -i "$PROFILE" | plutil -extract UUID raw -)"
for d in "$HOME/Library/MobileDevice/Provisioning Profiles" "$HOME/Library/Developer/Xcode/UserData/Provisioning Profiles"; do
  mkdir -p "$d"; cp "$PROFILE" "$d/$UUID.mobileprovision"
done
end

# --- archive, export, upload ----------------------------------------------------------------------------
step "Archive"
cd "$W/xc"
xcodebuild -project KGApp.xcodeproj -scheme KGApp -configuration Release -sdk iphoneos \
  -archivePath "$W/KG.xcarchive" archive \
  CODE_SIGN_STYLE=Manual CODE_SIGN_IDENTITY="Apple Distribution" DEVELOPMENT_TEAM="$TEAM_ID" \
  PROVISIONING_PROFILE_SPECIFIER="kg $ID appstore" OTHER_CODE_SIGN_FLAGS="--keychain $KC" \
  KG_PRODUCT_NAME="$PRODUCT" KG_BUNDLE_ID="$BUNDLE" KG_DISPLAY_NAME="$NAME" \
  KG_VERSION="$VER" KG_BUILD="$BUILD" KG_TEAM="$TEAM_ID" KG_PROFILE="kg $ID appstore" \
  KG_RUST_BIN="$BIN" > "$W/xcodebuild.log" 2>&1 || { grep -E 'error|\*\*' "$W/xcodebuild.log" | tail -30; exit 1; }
test -d "$W/KG.xcarchive/Products/Applications/$PRODUCT.app"
end

step "Export IPA"
cat > "$W/export.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>method</key><string>app-store-connect</string>
  <key>destination</key><string>export</string>
  <key>teamID</key><string>$TEAM_ID</string>
  <key>signingStyle</key><string>manual</string>
  <key>signingCertificate</key><string>Apple Distribution</string>
  <key>provisioningProfiles</key><dict><key>$BUNDLE</key><string>kg $ID appstore</string></dict>
  <key>uploadSymbols</key><false/>
  <key>manageAppVersionAndBuildNumber</key><false/>
</dict></plist>
PLIST
xcodebuild -exportArchive -archivePath "$W/KG.xcarchive" -exportPath "$W/out" -exportOptionsPlist "$W/export.plist"
IPA="$(ls "$W"/out/*.ipa)"; ls -la "$IPA"
end

step "Publish signed IPA"
# The App Store Connect key stays off this public runner; the IPA is uploaded with the API elsewhere.
cp "$IPA" "$W/$ID.ipa"
printf '{"id":"%s","version":"%s","build":"%s","product":"%s","bundle":"%s","upstream":"%s","signed":true}\n' \
  "$ID" "$VER" "$BUILD" "$PRODUCT" "$BUNDLE" "$(cat "$W/upstream.txt")" > "$W/$ID.json"
TAG="ipas-${GITHUB_RUN_NUMBER:-0}"
gh release view -R "$GITHUB_REPOSITORY" "$TAG" >/dev/null 2>&1 || gh release create -R "$GITHUB_REPOSITORY" "$TAG" --title "Signed iOS builds, run ${GITHUB_RUN_NUMBER:-0}" \
  --notes "App Store-signed IPAs from the testflight workflow (they only install through TestFlight)." --prerelease >/dev/null 2>&1 || true
ok=; for i in 1 2 3 4 5; do gh release upload -R "$GITHUB_REPOSITORY" "$TAG" "$W/$ID.ipa" "$W/$ID.json" --clobber && { ok=1; break; }; sleep $((i * 5)); done; [ -n "$ok" ]
echo "published $ID.ipa ($NAME $VER, build $BUILD) to release $TAG"
end
