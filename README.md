# Accessible Android Emulator (AAE)

AAE lets blind people create, run and test Android virtual devices without sighted help. Each device has a screen reader on from its first boot, takes your keyboard, and plays its audio reliably.

This is an early development build. It has a cross-platform core, the `aae` command line, and a Mac app.

## What works now

### Setting up

- No Android Studio needed. On first run, the Mac app shows what it needs: Google's Android emulator, platform tools (adb) and build tools, about 500 MB. It shows Google's licence for you to accept, then downloads each one, checks it against Google's checksum, and installs it as Android Studio's SDK manager would. `aae setup` does the same on the command line.
- Before downloading, AAE checks the computer can run the emulator at full speed (Hypervisor.framework on a Mac, KVM on Linux), and says what to do if it can't.
- AAE uses the Android SDK you already have, if there is one, such as Android Studio's. Otherwise it sets up its own, in its data folder. Set `ANDROID_HOME` to choose another place.
- Updating the emulator and platform tools AAE installed to Google's newest stable versions, from the Android Versions window or with `aae setup --update`. Every device has to be stopped first. AAE mentions available updates when it starts, at most once a day. Tools something else installed, such as Android Studio, are left for it to update; AAE marks the ones it installs with a `.installed-by-aae` file to tell them apart.

### Android versions

- Finding your Android SDK and the Android versions installed in it. `aae images` lists them with their size and the devices that use each one.
- Downloading Android versions. `aae available` lists every version Google offers for this computer, Android 5 and later, and `aae download --api 35` installs one into the SDK, the way Android Studio does. `aae create` downloads the version you ask for if it isn't installed. Google's licence is never accepted for you: the command line saves it for you to read, then you accept it with `--accept-licence`. The Mac app shows it in a sheet with Accept and Decline, and Decline is the default.
- Deleting Android versions to free disk space, with `aae remove-image --api 35` or the Mac app's Android Versions window. A version can't be deleted while any of AAE's devices use it. Android Studio shares the SDK, so AAE also names any Android Studio devices that use the version, and warns that they won't start without it.

### Devices

- Named devices, including several of the same Android version, each with its own apps and data: create, clone, rename and delete.
- Wiping a device back to how it was first set up (Device, Wipe, or `aae wipe`): its apps, data and snapshots go, and its screen reader is set up again. It keeps its name, hardware and volume. AAE keeps a copy of each screen reader it installs for this.
- Starting a device with no emulator window. Stopping it saves its state for a quick start next time, after first writing everything to its disk. `aae restart` restarts Android on a running device without losing anything on it.
- First-boot setup: hardware keyboard on, setup wizard skipped, screen kept awake, animations off, and a screen reader installed and turned on.
- Installing a new screen reader build on several devices, for screen reader developers. Installing a new build of the same screen reader keeps its settings. Stopped devices get it when they next start. In the Mac app, Install Screen Reader Build (Option-Shift-Command-I) chooses the devices, starting with the ones that use that screen reader; on the command line, `aae screen-reader all build.apk` does every device that uses it. A build signed differently can only replace the old one, which loses its settings, so AAE asks first (`--replace` on the command line).
- Choosing a screen reader. If the Android image has no screen reader and you didn't choose one, AAE asks whether to download Backtalk (its latest development build, checked against Backtalk's signing key), install your own APK, or go without. `aae screen-reader <device> backtalk` adds Backtalk later.
- Keeping accessibility services on. Every start, and every app install, turns back on any service Android turned off.
- Installing apps. The first time an app has an accessibility service, a keyboard, a notification listener or a device administrator, AAE asks whether to turn each one on, and remembers the answer for that device, so a reinstall or update applies it again without asking. `aae install` asks in the terminal; `--yes` turns them all on and `--no-services` leaves them off.
- Installing on several devices at once. In the Mac app, Install App (Command-I), dropping APKs on the window, or copying them in Finder and pasting them into the window asks which running devices to install on; `aae install` takes devices separated by commas. An app's parts are asked about once, and the answers used on every device.
- Sending keys and text, rotating, battery, location, text messages, calls, clipboard, screenshots, and named snapshots with notes (`aae snapshot <device> save "Before sign-in" --notes "…"`, then list, load, rename or delete).

### Speech and sound

- Playing the device's audio through AAE's own audio code, not the emulator's. The emulator's own audio output is off, which removes its crackle.
- Turning the screen reader's volume up to full on each new device, through AAE's helper app. Pass `--no-volume-boost` to leave it alone, or use `aae volume` to change it.
- Dependable speech. During setup and on every start, AAE's helper checks that the device's speech engine can actually speak. If Google's engine has downloaded voices that don't work, AAE resets it. If speech still fails, or the image has no engine, AAE installs its own build of eSpeak NG and makes it the default. That build speaks as soon as it's installed. `aae speech` runs the check by hand.
- Checking the sound. The Mac app watches that each device's sound is reaching it: when Android plays something and nothing arrives, it says so, once. Check Audio (Option-Command-K) also has AAE's helper play a test tone, which AAE listens for with its own playback muted so you don't hear it, and if the sound isn't getting through, offers to restart AAE's audio without restarting the device. `aae sound-check` does the same test in the terminal.
- Correct pitch on older Android. In the emulator, Android 10 and earlier play audio about 8% slow, so they sound low. AAE measures each device's audio speed silently during setup, and raises the pitch back when playing. Turn this off in the Mac app's Settings, or with `--no-pitch-correction`. `aae audio-check` measures again.

### Keyboard

- A full PC keyboard in Android. The emulator's own keyboard layout is a phone layout with no Meta key, so TalkBack's keyboard shortcuts can't work, and Escape, Home and End act as phone buttons. AAE's helper supplies a full keyboard layout and selects it every time a device starts. Tested on Android 8, 11, 14 and 16.
- `aae keytest`: checks that 15 kinds of key, Meta included, reach Android as the keys pressed.
- `aae latency`: measures the time from a key press to the device's speech.
- `aae attach`: the device's audio plays and your terminal's keyboard goes to the device. Control-] brings it back. Plain Escape goes to the device. A terminal can't see the Command key, so Option is sent as Android's Meta key, the modifier TalkBack's current keymap uses. Pass `--keep-alt` to send it as Alt. macOS Terminal turns Option-Left and Option-Right into word movement before AAE sees them. To fix that, open Terminal's Settings, then Profiles, then Keyboard. Turn on "Use Option as Meta key". Set Option-Left to send `\033[1;3D` and Option-Right to send `\033[1;3C`. iTerm2, Ghostty, kitty and WezTerm report every key and need none of this. The Mac app captures keys directly, with none of these limits.

### Gestures

- Screen reader gestures, performed with simulated fingers through the emulator's touchscreen, so the screen reader sees them as real touches: swipes in four directions, two-part swipes such as up then left, swipes with two, three or four fingers, single, double and triple taps, and double tap and hold. They follow the way the screen is turned. Tested with Backtalk on Android 8, 14 and 16.
- Gesture mode in the Mac app (Command-Shift-G): the whole keyboard performs gestures until Control-Command-Escape.
  - Arrows swipe; hold one arrow and press another for a two-part swipe.
  - Space double taps, T taps, and R triple taps. H double taps and holds, and L touches and holds, for as long as the key is held.
  - Hold 2, 3 or 4 while pressing a key to use that many fingers.
  - Gestures happen at the touch point, which starts in the middle of the screen. Tab and Shift-Tab move it to the next or previous thing on the screen, read out through VoiceOver. Shift-arrows move it a step at a time, saying what's under it. C puts it back in the middle, and W says where it is. This is how to tap a particular control in an app with no screen reader running, or in a part of an app a screen reader can't see. AAE's helper reads the screen for this, so it works whether or not the device has a screen reader on.
  - Question mark reads the keys out.
- `aae gesture <device> swipe-right double-tap` performs gestures in order; with no gestures it lists them all. `--at X,Y` performs them at a point, in the pixel positions the accessibility inspector reports. `aae inspect <device> --targets` lists the things gesture mode's Tab visits, with their positions.

### Testing tools

These are in the Mac app's Device menu and on the command line. The inspector, checks and speech log were checked on Android 8, 11, 14 and 16.

- Accessibility Inspector (Option-Command-I, `aae inspect`): the screen's accessibility tree, each element described the way a screen reader says it, with every property in its details. View it as a tree or a flat list, copy it as text, or save it as text or JSON.
- Accessibility checks (in the inspector, `aae check`): unlabelled controls, images without descriptions, small touch targets and duplicate labels on the current screen.
- Speech Log (Option-Command-L, `aae speech-log`): what the screen reader said, with times. While recording, speech goes through AAE's helper on its way to the real speech engine, which adds about 10 milliseconds. Turning recording on or off restarts the screen reader.
- Device Log (Option-Command-J, `aae logs`): the device's log, one line per row, filtered by app, tag, level and text. It can read out new errors as they happen, and it can be paused, copied and saved. `aae logs --follow` keeps showing new lines.
- Shell (Option-Command-T, `aae shell`): runs a command on the device and shows everything it printed, with its exit status. Commands are stopped after two minutes.

### The Mac app

The Mac app is in `macos/`. It updates itself: it checks for new versions, and AAE menu, Check for Updates, checks straight away. Announcements go through VoiceOver when it's running, and otherwise through the Mac's system voice. A short sound plays before each one; turn that off in Settings.

- The main window lists your devices. New Device (Command-N) creates one, downloading its Android version if needed.
- The File menu also has Android Versions (Option-Command-A), which lists installed versions with their size and the devices that use each one, and deletes the ones you no longer need.
- The Device menu starts (Command-Shift-S), stops (Command-Shift-.) and restarts (Command-Shift-R) the selected device. Speak Status (Command-Shift-I) says what it's doing. The menu also has Android's buttons, notifications and quick settings, rotation, muting, installing apps, screenshots, renaming, copying and deleting, and the testing tools.
- Each device has its own volume, remembered: a slider in the main window, and Turn Device Audio Up and Down (Option-Command-Up and Down) in the Device menu, alongside Mute (Command-Shift-M). `aae playback-volume` sets it in the terminal.
- Clipboard: Command-Shift-C copies the device's clipboard to the Mac, Command-Shift-V sends the Mac's clipboard to the device, and Option-Command-V types it on the device as key presses, for fields that block pasting.
- Snapshots (Option-Command-S): save the device as it is, with a name and notes, and restore, rename or delete snapshots later. Each one lists when it was taken.
- Battery, Location and Phone (Option-Command-B): set the battery level and charging, set the location from an address, a place name or coordinates, send the device a text message, and play the other end of a phone call: call the device, hang up, hold, or answer or be busy for a call the device makes.
- Gesture mode (Command-Shift-G) turns the keyboard into screen reader gestures, described under Gestures above.
- Device mode (Command-Shift-E) gives the keyboard to Android, from any AAE window. Every key goes to Android, with Command as Meta. That includes system shortcuts such as Spotlight's Command-Space and Mission Control. VoiceOver's own shortcuts, such as Command-F5, still work. Control-Command-Escape, or the "Return to the Mac" button, brings the keyboard back.

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

AAE finds the built helper and eSpeak NG on its own when you run it from this folder. Alternatively, put them next to the `aae` program as `aae-helper.apk` and `aae-espeak.apk`, or set `AAE_HELPER_APK` and `AAE_ESPEAK_APK` to their paths.

## Getting started

AAE sets up everything it needs itself. Open the Mac app and follow the setup screen, or on the command line:

```sh
aae setup
aae available
aae create "Android 16 test" --api 36 --backtalk
aae attach "Android 16 test"
aae stop "Android 16 test"
```

`aae help` lists every command, and `aae help <command>` explains one.

To set a default screen reader, so you don't have to choose one each time, put the APK at:

- macOS: `~/Library/Application Support/io.github.aaron-gh.AAE/screen-readers/default.apk`

Alternatively, set `AAE_SCREEN_READER_APK` to its path.

## Where things are kept

Devices live in AAE's data folder, under `devices`. On macOS that's `~/Library/Application Support/io.github.aaron-gh.AAE`. Set `AAE_HOME` to keep them somewhere else, such as an external drive. Each device is an ordinary emulator AVD folder plus an `aae.toml` file. To use AAE's devices from Android Studio, point `ANDROID_AVD_HOME` at the folder.

Android versions are installed into the Android SDK, where Android Studio sees them too.

## Diagnosing problems

- To report a bug, attach a diagnostic report: Help, Save Diagnostic Report in the Mac app, or `aae report`. It's plain text: AAE's version, this computer, the SDK, your devices and the end of their emulator logs, and AAE's own log. Your home folder, computer name and full name are taken out, and it never includes what you typed on a device, your clipboard, or the device's own log.
- AAE keeps its log in its data folder, under `logs`, at most about 4 megabytes. It records AAE's steps and problems, never keys, typed text or clipboards.
- `AAE_LOG=info aae start "My device"` shows each step of starting a device. Use `debug` for more.
- `AAE_KEYLOG=1` records every key the Mac app captures. Key codes reveal what you type, so leave it off otherwise.
- `crates/aae-core/examples/audio_probe.rs` prints how loud the device's audio stream is, in quarter seconds, while pressing keys.

## Layout

- `crates/aae-core`: the cross-platform core. It covers the SDK, downloading Android versions, the device store, the emulator, gRPC control, adb, audio, keys, speech, provisioning, the accessibility inspector and the device log.
- `crates/aae-cli`: the `aae` command.
- `crates/aae-ffi`: the bridge from the core to the host apps, generated with UniFFI.
- `macos`: the Mac app, in Swift.
- `crates/aae-core/proto`: the Android Emulator's gRPC definitions, under Apache 2.0.
- `android/helper`: AAE's helper app. It is an accessibility service, because Android lets only accessibility services set the accessibility volume and read the screen for the inspector. It also carries the full keyboard layout, a small tool AAE runs as the shell user to select it, the speech check, the speech log's relay engine, and the silent test tone used to measure audio speed.
- `android/espeak`: how AAE builds eSpeak NG: its own app ID, signed like the helper, with eSpeak NG's code unchanged.
- `android/third_party/espeak-ng`: eSpeak NG's source, as a git submodule.

## Licence

AAE is under the [Apache License 2.0](LICENSE). Parts it includes keep their own licences:

- The Android Emulator's gRPC definitions, in `crates/aae-core/proto`, are under Apache 2.0.
- eSpeak NG, in `android/third_party/espeak-ng`, is under the GNU General Public License version 3. AAE builds it unchanged as a separate app that runs on the Android device.
- Sparkle, which updates the Mac app, is under the MIT licence.
