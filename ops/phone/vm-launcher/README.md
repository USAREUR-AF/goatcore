# goat node launcher (Android)

| Field | Value |
|---|---|
| Status | **WIP, not for production use.** Measured once on one phone (a Google Pixel 10a, Android 16), 2026-11-14: after a reboot and one unlock the VM was running 20 s later with no other action. Signed with a throwaway key for that test; it has no release. Gate: row H8 of `docs/design/goatbuilder-v1-gate.md` |

A phone node runs in the Debian VM that Android's Linux Terminal app starts
(`docs/design/phone-node.md`). After a phone reboot nothing opens that app.
This app receives Android's boot broadcast and opens the Terminal app, which
starts the VM.

Android delivers the boot broadcast after the phone's storage is unlocked. On
a phone with a lock-screen PIN that is the first unlock by a person; on a
phone with no lock screen it is at boot.

## Build

Needs the Android SDK build tools and platform 36, and a JDK.

```
BT=$ANDROID_HOME/build-tools/36.0.0; AJ=$ANDROID_HOME/platforms/android-36/android.jar
mkdir -p out/classes
$BT/aapt2 compile --dir res -o out/res.zip
$BT/aapt2 link -o out/base.apk -I $AJ --manifest AndroidManifest.xml out/res.zip
javac --release 17 -cp $AJ -d out/classes src/org/goat/vmlauncher/*.java
$BT/d8 --lib $AJ --min-api 35 --output out out/classes/org/goat/vmlauncher/*.class
(cd out && zip -q base.apk classes.dex)
$BT/zipalign -f 4 out/base.apk out/aligned.apk
$BT/apksigner sign --ks <your keystore> --out out/goat-vmlauncher.apk out/aligned.apk
```

## Install

```
adb install -r out/goat-vmlauncher.apk
adb shell appops set org.goat.vmlauncher SYSTEM_ALERT_WINDOW allow   # lets it open an app from the background
adb shell dumpsys deviceidle whitelist +org.goat.vmlauncher
adb shell am start -n org.goat.vmlauncher/.MainActivity               # once: Android sends no broadcast to an app never opened
```

After opening it once, wait a minute before any reboot: Android writes the
"has been opened" state to disk some seconds later, and an app that reads as
never opened after the reboot receives no boot broadcast.
