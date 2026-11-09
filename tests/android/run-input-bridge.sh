#!/usr/bin/env bash
# An already-booted disposable x86_64 emulator is required.
set -euo pipefail
: "${ANDROID_SERIAL:?Set ANDROID_SERIAL to a disposable x86_64 emulator}"
[[ $ANDROID_SERIAL == emulator-* ]] || { echo 'Emulator only' >&2; exit 1; }

tests=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$tests/../.." && pwd)
sdk=${ANDROID_SDK_ROOT:-$HOME/android-sdk}
tools=$sdk/build-tools/35.0.0
jar=$sdk/platforms/android-35/android.jar
clang=$sdk/ndk/27.2.12479018/toolchains/llvm/prebuilt/linux-x86_64/bin/x86_64-linux-android29-clang
out=$(mktemp -d)
trap '"$sdk/platform-tools/adb" uninstall io.github.xpjb.chad.inputtest >/dev/null 2>&1 || true; rm -rf "$out"' EXIT
mkdir -p "$out/classes" "$out/dex"
cat > "$out/AndroidManifest.xml" <<'XML'
<manifest xmlns:android="http://schemas.android.com/apk/res/android" package="io.github.xpjb.chad.inputtest">
<uses-sdk android:minSdkVersion="29" android:targetSdkVersion="35"/>
<application android:theme="@android:style/Theme.Material.NoActionBar" android:extractNativeLibs="true" android:debuggable="true">
<activity android:name="io.github.xpjb.chad.ChadActivity" android:exported="true">
<meta-data android:name="android.app.lib_name" android:value="chad_input_test"/>
</activity>
</application>
<instrumentation android:name="io.github.xpjb.chad.test.InputBridgeTest" android:targetPackage="io.github.xpjb.chad.inputtest"/>
</manifest>
XML
"$clang" -shared -fPIC -std=c11 "$tests/native-input-stub.c" -landroid \
    -o "$out/libchad_input_test.so"
javac -source 8 -target 8 -Xlint:-options -classpath "$jar" -d "$out/classes" \
    "$repo/android/java/io/github/xpjb/chad/ChadActivity.java" \
    "$tests/InputBridgeTest.java"
mapfile -t classes < <(find "$out/classes" -name '*.class' | sort)
"$tools/d8" --min-api 29 --lib "$jar" --output "$out/dex" "${classes[@]}"
"$tools/aapt2" link -I "$jar" --manifest "$out/AndroidManifest.xml" -o "$out/test.apk"
python3 - "$out" <<'PY'
import sys, zipfile
from pathlib import Path
path = Path(sys.argv[1])
with zipfile.ZipFile(path / 'test.apk', 'a', compression=zipfile.ZIP_DEFLATED) as archive:
    archive.write(path / 'libchad_input_test.so', 'lib/x86_64/libchad_input_test.so')
    archive.write(path / 'dex/classes.dex', 'classes.dex')
PY
"$tools/zipalign" -f 4 "$out/test.apk" "$out/aligned.apk"
"$tools/apksigner" sign --ks "$HOME/.android/debug.keystore" \
    --ks-key-alias androiddebugkey --ks-pass pass:android --key-pass pass:android \
    --out "$out/signed.apk" "$out/aligned.apk"
"$sdk/platform-tools/adb" install -r "$out/signed.apk"
timeout 60 "$sdk/platform-tools/adb" shell am instrument -w \
    io.github.xpjb.chad.inputtest/io.github.xpjb.chad.test.InputBridgeTest | tee "$out/result"
grep -q 'PASS: toolbar, stale-generation fence, batch commits and same-generation IME ownership' \
    "$out/result"
