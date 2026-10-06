# Accessible Android Emulator (AAE)

AAE lets blind people create, run and test Android virtual devices without sighted help. Each device has a screen reader on from its first boot, takes your keyboard, and plays its audio reliably.

This is an early development build. It has a cross-platform core, the `aae` command line, and a first Mac app.

## What works now

- Finding your Android SDK and the Android versions installed in it.
- Named devices, including several of the same Android version: create, clone, rename and delete.
- Starting a device with no emulator window, and stopping it with its state saved.
- First-boot setup: hardware keyboard on, setup wizard skipped, screen kept awake, and a screen reader installed and turned on.
- Keeping accessibility services on. Every start, and every app install, turns back on any service Android turned off.
- Installing apps, and turning on the accessibility services inside them.
- Playing the device's audio through AAE's own audio code, not the emulator's. The emulator's own audio output is off, which removes its crackle.
- Turning the screen reader's volume up to full on each new device, through AAE's helper app. Pass `--no-volume-boost` to leave it alone, or use `aae volume` to change it.
- Sending keys and text, rotating, battery, location, text messages, calls, clipboard, screenshots and snapshots.
- `aae attach`: the device's audio plays and your terminal's keyboard goes to the device. Control-] brings it back. Plain Escape goes to the device. A terminal can't see the Command key, so Option is sent as Android's Meta key, the modifier TalkBack's current keymap uses. Pass `--keep-alt` to send it as Alt. macOS Terminal turns Option-Left and Option-Right into word movement before AAE sees them. To fix that, open Terminal's Settings, then Profiles, then Keyboard. Turn on "Use Option as Meta key". Set Option-Left to send `\033[1;3D` and Option-Right to send `\033[1;3C`. iTerm2, Ghostty, kitty and WezTerm report every key and need none of this. The Mac app captures keys directly, with none of these limits.
- `aae latency`: measures the time from a key press to the device's speech.
- A full PC keyboard in Android. The emulator's own keyboard layout is a phone layout with no Meta key, so TalkBack's keyboard shortcuts can't work, and Escape, Home and End act as phone buttons. AAE's helper supplies a full keyboard layout and selects it every time a device starts. Tested on Android 11, 14 and 16.
- `aae keytest`: checks that 15 kinds of key, Meta included, reach Android as the keys pressed.
- `aae restart`: restarts Android on a running device without losing anything on it.
- Dependable speech. During setup and on every start, AAE's helper checks that the device's speech engine can actually speak. If Google's engine has downloaded voices that don't work, AAE resets it. If speech still fails, or the image has no engine, AAE installs its own build of eSpeak NG and makes it the default. That build speaks as soon as it's installed. `aae speech` runs the check by hand.
- The Mac app (`macos/`): a device list, a New Device sheet, and a Device menu with shortcuts for starting, stopping and Android's buttons. In device mode (Command-Shift-E) every key goes to Android, with Command as Meta, until Control-Command-Escape brings the keyboard back. Announcements go through VoiceOver when it's running, and otherwise through the Mac's system voice.

## Building

You need Rust (from [rustup.rs](https://rustup.rs)) and CMake 3.24 or later. You don't need protoc.

```sh
cargo build --release
```

The command is then `target/release/aae`.

AAE's helper app is an Android app that runs on each device. It needs JDK 17 and the Android SDK:

```sh
cd android
./gradlew :helper:assembleRelease
```

The Mac app needs Xcode, or its command-line tools, and is built with:

```sh
macos/build.sh
```

That builds the Rust core, generates the Swift bindings, builds the helper app and eSpeak NG, and puts `macos/build/AAE.app` together.

eSpeak NG is built from its source, a git submodule in `android/third_party/espeak-ng`, by `android/build-espeak.sh`. The first build takes a few minutes and downloads the Android NDK version it needs. Clone AAE with `--recurse-submodules`, or the script fetches the submodule itself. eSpeak NG is under GPL v3.

AAE finds the built helper on its own when you run it from this folder. Alternatively, put it next to the `aae` program as `aae-helper.apk`, or set `AAE_HELPER_APK` to its path.

## Getting started

You need the Android SDK with the emulator, platform tools, build tools, and at least one system image. Android Studio installs these. Downloading Android versions from inside AAE is coming.

```sh
aae doctor
aae images
aae create "Android 16 test" --api 36 --screen-reader path/to/backtalk.apk
aae attach "Android 16 test"
aae stop "Android 16 test"
```

`aae help` lists every command, and `aae help <command>` explains one.

To set a default screen reader, so you don't have to pass `--screen-reader` each time, put the APK at:

- macOS: `~/Library/Application Support/io.github.aaron-gh.AAE/screen-readers/default.apk`

Alternatively, set `AAE_SCREEN_READER_APK` to its path.

## Where things are kept

Devices live in AAE's data folder, under `devices`. Set `AAE_HOME` to keep them somewhere else, such as an external drive. Each device is an ordinary emulator AVD folder plus an `aae.toml` file. To use AAE's devices from Android Studio, point `ANDROID_AVD_HOME` at the folder.

## Layout

- `crates/aae-core`: the cross-platform core. It covers the SDK, the device store, the emulator, gRPC control, adb, audio, keys and provisioning.
- `crates/aae-cli`: the `aae` command.
- `crates/aae-ffi`: the bridge from the core to the host apps, generated with UniFFI.
- `macos`: the Mac app, in Swift.
- `crates/aae-core/proto`: the Android Emulator's gRPC definitions, under Apache 2.0.
- `crates/aae-core/examples/audio_probe.rs`: prints how loud the device's audio stream is, in quarter seconds, while pressing keys. Use it to diagnose audio problems.
- `android/espeak`: how AAE builds eSpeak NG: its own app ID, signed like the helper, with eSpeak NG's code unchanged.
- `android/helper`: AAE's helper app. It is an accessibility service, because Android lets only accessibility services set the accessibility volume. It also carries the full keyboard layout, and a small tool AAE runs as the shell user to select it.
