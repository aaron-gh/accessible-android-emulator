# Accessible Android Emulator (AAE)

Create, run and test Android virtual devices with a screen reader. Each device has a screen reader on from first boot, takes the whole keyboard, and plays its audio through AAE.

Mac and Windows apps, AAE Remote for Android phones, the `aae` command line, and an MCP server for AI agents, on one core. Early development: the Mac app is the most complete; the Windows app is new.

## Getting AAE

- **Mac:** the disk image from the [latest release](https://github.com/aaron-gh/accessible-android-emulator/releases/latest). Apple silicon, macOS 13 or later. Not notarised: on first open, System Settings, Privacy & Security, Open Anyway.
- **Windows:** `AAE-<version>-windows-x64-setup.exe` from the latest release. Installs per user, no administrator prompt. Windows 10 or 11, 64-bit, Intel or AMD, with Windows Hypervisor Platform on. No Windows on ARM (Google's emulator doesn't support it). Not signed: SmartScreen, More info, Run anyway.
- **Android phones:** `AAE-Remote-<version>.apk`. See [AAE Remote](#aae-remote).
- **Development builds:** every change to `master`, in the [dev prerelease](https://github.com/aaron-gh/accessible-android-emulator/releases/tag/dev): [AAE-dev.dmg](https://github.com/aaron-gh/accessible-android-emulator/releases/download/dev/AAE-dev.dmg), [AAE-dev-windows-x64-setup.exe](https://github.com/aaron-gh/accessible-android-emulator/releases/download/dev/AAE-dev-windows-x64-setup.exe). Help, About AAE shows the build.

Both apps update themselves. Check for Updates offers stable releases only.

### System requirements

Below these, Android starts but is too slow to use with a screen reader. Setup and the self-test report it.

- **Processor:** 6 or more threads (recent Intel Core i5, AMD Ryzen 5, any Apple silicon). Intel N100, N150, N200, Celeron, Pentium Silver and Atom are too slow. Each device gets half the threads, up to 4.
- **Memory:** 8 GB; 16 GB for more than one device.
- **Graphics:** the computer's graphics adapter. If the emulator can't use it, AAE switches that device to software rendering. `AAE_GPU` overrides it with an emulator `-gpu` mode, such as `swiftshader_indirect`.
- **Disk:** SSD; about 10 GB for the emulator, one Android version and a device, plus about 2 GB per further version.

## Features

### Setup

- No Android Studio needed. AAE downloads Google's emulator, platform tools and build tools (about 500 MB) after showing Google's licence, and checks each against Google's checksum. `aae setup`.
- Checks hardware virtualisation first: Hypervisor.framework, Windows Hypervisor Platform or KVM.
- Uses an existing Android SDK if there is one, otherwise its own in its data folder. `ANDROID_HOME` sets another.
- Updates the tools it installed (marked with `.installed-by-aae`), with every device stopped; reports available updates at most once a day. `aae setup --update`. Tools installed by something else are left alone.

### Android versions

- `aae available` lists every version Google offers for the computer, Android 5 and later, including minor releases such as API 36.1. New Device, `aae create` and `aae download` install into the SDK. `--api` takes `37`, `36.1` or a preview's name.
- Previews (betas, Canary) only on request: Include Previews in New Device, or `--previews`. 16 KB page images are marked.
- Google's licence is never accepted automatically. In the apps, Escape declines and Enter does nothing. The CLI saves the licence and takes `--accept-licence`.
- Installed versions are listed with size and the devices using them. A version used by an AAE device can't be deleted; Android Studio devices using it are named.

### Devices

- Create, copy, rename, wipe and delete. Sizes: small phone, phone, tablet, and foldable (folds in, 337 dp wide folded and 674 dp unfolded). Wipe resets to first setup and keeps name, hardware and volume.
- Hardware (stopped devices): memory, cores, storage, screen size and density, from next start. `aae hardware <device> memory 4096 cores 6 storage 16G screen 1440x3120 density 560`; `cores auto` reverts to AAE's choice. Reducing storage needs a wipe.
- Export and Import: one `.aaedevice` file with apps, data, settings and named snapshots, between computers with the same processor architecture. The Android version must be installed on the target. `aae export`, `aae import`.
- No emulator window.
- Stop doesn't shut Android down: it saves the whole running device, open apps included, as its quick-boot snapshot. Start restores it in a few seconds.
- Restart reboots Android in the running emulator.
- Cold Boot starts Android from its disk without the quick-boot snapshot, and deletes the snapshot, stopping the device first if needed (`aae start --cold`). Apps and data stay.
- First boot: hardware keyboard on, setup wizard skipped, screen kept on, animations off, screen reader installed and on.
- No screen reader in the image: AAE offers Backtalk (latest development build, verified against Backtalk's signing key), an APK, or none.
- Screen reader builds install on several devices at once. Same signature keeps settings; stopped devices get it at next start. A different signature replaces the old app after confirmation.
- Accessibility services are turned back on after every start and install. Accessibility Services switches the screen reader (one at a time) and toggles other services.
- App installs to one or more devices. Accessibility services, keyboards, notification listeners and device administrators are offered for turning on; the choice is remembered per device. Watch for New Builds installs each new APK from a file or build folder.
- Apps: open, force stop, clear, uninstall; permissions and special access (battery, overlay, usage access, system settings) as switches, with Grant All.
- Links and intents, keys and text, rotation, clipboard both ways, screenshots, named snapshots with notes.

### Audio and speech

- Device audio is played by AAE from the emulator's gRPC stream; the emulator's own output is off. Per-device volume, mute and audio output. With several devices running, only the one in use plays (Settings).
- Screen reader volume is set to full on new devices. `--no-volume-boost` skips it; `aae volume` sets it.
- Text-to-speech is checked at setup and every start. Broken Google voices are reset; if speech still fails, AAE's eSpeak NG is installed and made default. `aae speech`.
- The apps report once if a device plays audio that doesn't arrive. Check Audio plays a muted test tone and offers to restart AAE's audio. `aae sound-check`.
- Microphone: Turn On Microphone sends the host microphone into the device while an app on it records, starting 500 ms after recording starts (earlier input crashes the emulator). The emulator never opens the host microphone itself. Check Microphone injects a test tone and reports whether it was recorded. Play Audio File into Microphone plays WAV, MP3, FLAC, Ogg Vorbis or M4A from the start of the next recording. `aae mic <device> on|check|level|play <file>`.
- Speech bridge: Turn On Speech Bridge sends the screen reader's speech to the host's text-to-speech instead of as audio. Mac: AVSpeechSynthesizer with VoiceOver's voice and rate when VoiceOver runs, otherwise Spoken Content's. Windows: NVDA 2024.1 or later, otherwise SAPI 5. Other languages use a voice for that language. Stops and completions are passed through, so continuous reading works. Earcons stay in device audio. Independent of the speech log. With no app connected, the device uses its own text-to-speech. `aae speech-bridge <device> on|off|listen`.
- Pitch correction for Android 10 and earlier, which play about 8% slow. Measured at setup. Off in Settings or with `--no-pitch-correction`. `aae audio-check` remeasures.

### Keyboard

- Device mode sends every key to Android, system shortcuts included, until the return shortcut. The host screen reader keeps its keys.
- AAE's helper selects a full PC keyboard layout at every start, so Meta, Escape, Home and End work as keys.
- Keyboard layouts for 52 languages, each Android's own with the PC keys above. The apps use the one for the computer's keyboard, and switch when it changes. On a Mac, keys typed without Command, Control or Option are sent as the characters the Mac typed, so they're right even where Mac and PC layouts differ, such as British. `aae keyboard <device> [layout|auto]` chooses one; `--list` lists them. `aae attach` follows the terminal's language.
- `aae keytest` checks 15 key types reach Android correctly. `aae latency` measures key press to speech.
- `aae attach` sends the terminal's keys and plays audio until Control-]. Option is sent as Meta (`--keep-alt` for Alt). macOS Terminal: Settings, Profiles, Keyboard, turn on Use Option as Meta key, and map Option-Left to `\033[1;3D` and Option-Right to `\033[1;3C`. iTerm2, Ghostty, kitty and WezTerm need no changes.

### Gestures

- Screen reader gestures through the emulator's touchscreen: swipes, two-part swipes, two to four fingers, single, double and triple taps, double tap and hold. They follow rotation.
- Gesture mode:
  - Arrows swipe; hold one arrow and press another for a two-part swipe.
  - Space double tap, T tap, R triple tap, H double tap and hold, L touch and hold.
  - Hold 2, 3 or 4 for that many fingers.
  - Touch point: Tab and Shift-Tab move between screen elements, Shift-arrows move in steps, C centres, W says what's there and its position in percent and pixels, D reads every property of the element there. Works without a screen reader running.
  - Question mark lists the keys.
- `aae gesture <device> swipe-right double-tap`; `--at X,Y`. `aae inspect <device> --targets` lists touch targets; `--at X,Y` prints the element at a point.

### Battery, Location, Phone and Network

- Battery level, charging and health (good, failed, dead, over voltage, overheated).
- Fingerprint touches, fingers 1 to 10. Enroll them in Android's security settings.
- Shake.
- Fold and unfold a foldable device.
- Location from coordinates, a place or an address (looked up on OpenStreetMap; only the typed text is sent).
- Play GPX Route: moves the location once a second along the file's track, route or waypoints, at the file's pace or 30 km/h, with a speed multiplier. Choose again to stop.
- Text messages to the device; incoming calls, and answer, busy or hang up for outgoing calls.
- Network: airplane mode, Wi-Fi, mobile data, and speed (full, LTE, 3G, slow 3G, EDGE, GPRS).

CLI: `battery`, `fingerprint`, `shake`, `fold`, `unfold`, `location`, `route`, `sms`, `call`, `network`.

### Display and Language

Language, font size, display size, dark theme (Android 10+), bold text (Android 12+), high contrast text, colour inversion, colour correction, animations, captions and touch and hold delay. Languages include the pseudo-locales en-XA (accented, longer) and ar-XB (right to left), and any typed language tag. Use These Settings for New Devices applies them to devices created later. `aae settings <device> [name value]...`, `--list`, `--for-new-devices`, `--forget-new-devices`.

### Testing tools

- Accessibility Inspector (`aae inspect`): the accessibility tree as a screen reader describes it, with all properties. Tree or list; copy as text; save as HTML, JSON or text (`--html`). Follow the screen (`--follow`) refreshes when the screen changes.
- Accessibility checks (`aae check`): unlabelled controls, undescribed images, small touch targets, duplicate labels.
- Speech Log (`aae speech-log`): screen reader utterances with times. Adds about 10 ms.
- Device Log (`aae logs`): filter by app, tag, level and text; announce new errors; pause, copy, save. `--follow`.
- Shell (`aae shell`): runs a command as the shell user. Two-minute limit.
- Record Screen (`aae record`): screen and audio to WebM, up to three minutes.

## Mac app

`macos/`. Announcements go to VoiceOver, or the Spoken Content voice when VoiceOver is off, each after a sound (Settings).

- File: New Device (Command-N), Import Device, Android Versions (Option-Command-A), Serve Devices to Phones.
- Device:
  - Start (Command-Shift-S), Stop (Command-Shift-.), Restart (Command-Shift-R), Cold Boot, Use Android Keyboard (Command-Shift-E), Use Gestures (Command-Shift-G), Open in Own Window (Option-Command-O), Speak Status (Command-Shift-I).
  - Android buttons including Power and Assistant, rotation, Fold and Unfold, Hardware, Export, rename, copy, wipe, delete.
  - Accessibility Inspector (Option-Command-I), Speech Log (Option-Command-L), Device Log (Option-Command-J), Shell (Option-Command-T), Apps (Option-Command-P), Accessibility Services (Option-Command-U), Snapshots (Option-Command-S), Battery, Location, Phone and Network (Option-Command-B), Display and Language (Option-Command-Comma).
  - Mute (Command-Shift-M), volume (Option-Command-Up and Down), Check Audio (Option-Command-K), Turn On Microphone (Command-Shift-U), Turn On Speech Bridge.
  - Clipboard: device to Mac (Command-Shift-C), Mac to device (Command-Shift-V), type the Mac clipboard (Option-Command-V).
  - Record Screen (Option-Command-R), Save Screenshot.
  - Install App (Command-I), or drop or paste APKs. Install Screen Reader Build (Option-Shift-Command-I). Open Link (Command-Shift-L).
- Main window: volume and Audio output.
- Device mode: Command is Meta. Spotlight, Mission Control and other system shortcuts go to Android; VoiceOver shortcuts still work. Control-Command-Escape or the Return to the Mac button exits. Settings offers Control-Shift-Command-Escape or Control-Option-Command-Escape instead.
- Open in Own Window: the device whose window is in front is the selected device.

## Windows app

`crates/aae-windows`. Rust with standard Win32 controls, cross-compiled on the Mac.

Announcements go to NVDA (controller client, with braille), JAWS (`FreedomSci.JawsApi`), UI Automation notifications for other screen readers, or SAPI with no screen reader.

- Mac shortcuts with Control for Command and Alt for Option. Rename is F2, Delete is Delete.
- Device mode: the Windows key is Meta. Alt-Tab, the Windows key and Alt-F4 go to Android; Control-Alt-Delete and Windows-L can't. Keys held with Insert or Caps Lock go to the host screen reader. Control-Windows-Escape or the Return to Windows button exits; Settings offers Control-Shift-Windows-Escape or Control-Alt-Windows-Escape.
- Device, Audio Output; `aae audio-output <device> <name>`. Falls back to the system default when the output is missing.
- Tool windows as on the Mac: Tab moves, F5 refreshes, Escape closes.
- In a device's own window, menu shortcuts act on that device; device and gesture mode run in the main window.

## AAE Remote

Android app (`android/remote`; server `crates/aae-remote`) for the devices on a Mac or PC. Audio and vibration play on the phone; keys and touches go to the device. Android 8 or later; gesture mode needs Android 11.

- **Install:** `AAE-Remote-<version>.apk` from the latest release; `AAE-Remote-dev.apk` in the dev prerelease. If Play Protect blocks it: More details, Install anyway.
- **Updates:** checked daily and with Check for Updates, verified against AAE's update feed, confirmed by Android. Offer development builds opts in.
- **Serving:** File, Serve Devices to Phones, or `aae serve` (no display or audio device needed). Allow incoming connections if asked.
- **At login:** Serve at login, without AAE open (in Serve Devices to Phones) or `aae daemon install|uninstall|status`: a LaunchAgent, a Windows Run entry or a systemd user service. `aae pair` makes a pairing code for it. Logs to `serve.log`.
- **Pairing:** Pair with a Computer: choose a computer on the network or type an address, then enter its 12-character code (single use, 10 minutes). TLS with the certificate pinned. `aae phones`, `aae phones --unpair <id>`.
- **Managing:** devices (create, start, stop, restart, wipe, copy, rename, delete), Android versions (download after accepting the licence on the phone, delete), tool setup and updates.
- **Device screen:** Android buttons, Speak Status, rotation, Fold and Unfold, Restart, Cold Boot. Testing Tools: speech log, device log, inspector, apps, services, snapshots, Battery, Location, Phone and Network (including GPX upload), Display and Language, links and intents, clipboard, APK install.
- **Microphone:** sends the phone's microphone while an app on the device records (voice communication source, echo cancelled). Foreground only. Testing Tools uploads an audio file to play into it.
- **Speech bridge:** sends the device's speech to the phone's text-to-speech. Engine and rate: Bridge Text-to-Speech Settings (default: system). Takes priority over a desktop app while the phone is attached.
- **Tablets and unfolded phones:** on a window at least 600 dp wide, a screen opens beside the one it was opened from, such as a device's controls beside the device list. Gesture and keyboard mode take the whole window. Android 12L or later.
- **Keyboard mode:** a connected keyboard's keys go to the device. Control-Shift-Escape or a long press of volume down exits.
- **Gesture mode:** the phone's screen is the device's touchscreen, multi-finger gestures included. Long press of volume down exits; volume keys change the device's volume. Needs the AAE gesture mode accessibility service (Samsung: under Installed apps), which passes touches through only while gesture mode is open. Without it, buttons perform common gestures.
- **Touch point mode:** a long press of volume up in gesture mode switches to it and back. Two-finger swipe right or left: next or previous element. One-finger swipe: move a step. Touch and hold, then drag: the point goes where the finger is; with "Touch point drag moves the point from where it is, at half speed" on the Computers screen, it moves from where it was at half the finger's speed, for large screens. Tap or double tap: that gesture at the point; tap, then touch and hold: touch and hold there. Two-finger tap: what's there and its position in percent and pixels; double tap: every property; triple tap: back to the middle.
- **Restricted setting (Android 13+):** after one attempt to turn the service on: Settings, Apps, AAE Remote, More options, Allow restricted settings.

## Command line

`aae` ships with both apps: `/Applications/AAE.app/Contents/Helpers/aae`, `%LOCALAPPDATA%\Programs\AAE\aae.exe`. `aae help [command]`. Devices are referred to by name.

```sh
aae setup
aae available
aae create "Android 16 test" --api 36 --backtalk
aae attach "Android 16 test"
aae stop "Android 16 test"
```

- Setup: `setup`, `doctor`, `self-test`, `report`.
- Android versions: `available`, `download`, `images`, `remove-image`.
- Devices: `list`, `create`, `clone`, `rename`, `hardware`, `export`, `import`, `wipe`, `delete`, `start`, `restart`, `stop`, `status`, `snapshot`.
- Keyboard, audio and speech: `attach`, `keyboard`, `listen`, `playback-volume`, `audio-output`, `volume`, `key`, `type`, `gesture`, `keytest`, `latency`, `sound-check`, `audio-check`, `mic`, `speech`, `speech-bridge`.
- Screen readers and apps: `screen-reader`, `services`, `install`, `apps`, `app`, `watch`, `link`, `intent`.
- Testing: `inspect`, `check`, `speech-log`, `logs`, `shell`, `screenshot`, `record`.
- Conditions: `rotate`, `battery`, `fingerprint`, `shake`, `fold`, `unfold`, `location`, `route`, `network`, `settings`, `sms`, `call`, `clipboard`.
- AAE Remote: `serve`, `pair`, `phones`, `daemon`.
- AI agents: `mcp`.

## AI agents (MCP)

`aae mcp` is an MCP server for running, using and inspecting devices. The speech log gives agents what the screen reader announced, not only what's drawn.

- Observe: `screenshot`, `inspect`, `check_accessibility`, `touch_targets`, `speech_log`, `device_log`.
- Act: `gesture`, `tap_target`, `activate_target`, `type_text`, `press_key`, `open_app`, `open_link`, `send_intent`, notifications, quick settings, `rotate`, `device_settings`, battery, `touch_fingerprint`, `shake`, `fold`, location, `play_route`, `stop_route`, network, SMS, calls, clipboard, `play_into_microphone`, `start_recording`, `stop_recording`.
- Wait: `wait_for` text on screen or in speech, or its absence; `pause`.
- Set up: devices, `install_app`, apps, permissions, accessibility services, snapshots. No Android version downloads (the user accepts Google's licence in the app).
- Wipe, delete, delete snapshot, uninstall, clear data and shell need `aae mcp --allow-destructive`. Loading a snapshot is marked destructive.

```sh
claude mcp add aae -- /Applications/AAE.app/Contents/Helpers/aae mcp
claude mcp add aae -- "$env:LOCALAPPDATA\Programs\AAE\aae.exe" mcp   # Windows, PowerShell
```

Other agents: `{"mcpServers": {"aae": {"command": "/Applications/AAE.app/Contents/Helpers/aae", "args": ["mcp"]}}}`.

## Files

- Data folder: `~/Library/Application Support/io.github.aaron-gh.AAE` (Mac), `%LOCALAPPDATA%\aaron-gh\AAE\data` (Windows); `AAE_HOME` overrides. Devices are AVD folders under `devices`, each with an `aae.toml`; set `ANDROID_AVD_HOME` to that folder to use them in Android Studio.
- Android versions are installed in the Android SDK.
- Default screen reader: `screen-readers/default.apk` in the data folder, or `AAE_SCREEN_READER_APK`.

## Diagnostics

- Self-test (Help, Run Self-Test; `aae self-test`): virtualisation, audio output, SDK, AAE's parts, disk space, and each running device's screen reader, helper and speech. The apps also check keyboard capture and device audio. Silent.
- Diagnostic report (Help, Save Diagnostic Report; `aae report`): plain text with versions, computer, SDK, devices, emulator log tails and AAE's log. Home folder, computer name and (Mac) full name are removed. Typed text, clipboards and device logs are never included.
- AAE's log: `logs` in the data folder, about 4 MB at most. No keys, typed text or clipboards.
- `AAE_LOG=info` (or `debug`) prints each step. `AAE_KEYLOG=1` logs Mac key captures, which reveals typing.
- `crates/aae-core/examples/audio_probe.rs` prints the device audio level per quarter second.

## Building

Rust from [rustup.rs](https://rustup.rs). No protoc or CMake.

```sh
cargo build --release                                # target/release/aae
cd android && ./gradlew :helper:assembleRelease      # helper: JDK 17, Android SDK
macos/build.sh                                       # Mac app: Xcode or its command-line tools
brew install mingw-w64 makensis && windows/build.sh  # Windows app, cross-compiled on the Mac
android/build-remote.sh                              # AAE Remote
```

- `macos/build.sh` builds the core, Swift bindings, helper and eSpeak NG into `macos/build/AAE.app`.
- `windows/build.sh` adds the Rust Windows target if missing, and packages `AccessibleAndroidEmulator.exe`, `aae.exe`, the helper, eSpeak NG, NVDA's controller client, WinSparkle and a read-me into an installer and zip in `windows/dist`.
- eSpeak NG is built from the `android/third_party/espeak-ng` submodule by `android/build-espeak.sh`, which downloads the NDK. Clone with `--recurse-submodules`, or the script fetches it.
- Android signing: `~/.android/aae.keystore`, password in `AAE_ANDROID_KEY_PASSWORD` or the Mac Keychain item "AAE Android signing key"; otherwise the debug key. A device copy with another signature is reinstalled.
- AAE finds the built helper and eSpeak NG when run from this folder, next to `aae` as `aae-helper.apk` and `aae-espeak.apk`, or via `AAE_HELPER_APK` and `AAE_ESPEAK_APK`.
- Development builds: `.github/workflows/dev-build.yml`.

## Layout

- `crates/aae-core`: SDK, Android versions, device store, emulator, gRPC, adb, audio, keys, provisioning, inspector, device log.
- `crates/aae-cli`: `aae`.
- `crates/aae-mcp`: `aae mcp`.
- `crates/aae-remote`: `aae serve`, over TLS WebSocket.
- `crates/aae-ffi`: UniFFI bindings for the Mac app; used directly by the Windows app.
- `macos`: Mac app (Swift); `appcast.xml`.
- `crates/aae-windows`: Windows app. `windows`: build and release scripts, installer, read-me. `appcast-windows.xml`.
- `crates/aae-core/proto`: the emulator's gRPC definitions.
- `android/helper`: helper app, an accessibility service (needed for the accessibility volume and screen reading). Also the keyboard layout, a shell-user tool (keyboard layout, vibration and recording watches, system locale), speech check, speech relay and bridge, test tone.
- `android/remote`: AAE Remote (Kotlin) and its gesture mode service.
- `android/espeak`: eSpeak NG packaging: own app ID, code unchanged. Source: `android/third_party/espeak-ng`.

## Licence

[Apache License 2.0](LICENSE). Included parts keep their licences:

- Emulator gRPC definitions (`crates/aae-core/proto`): Apache 2.0.
- eSpeak NG: GPL v3, built unchanged as a separate app on the device.
- Sparkle (Mac updates): MIT.
- NVDA controller client (`nvdaControllerClient.dll`): LGPL 2.1, downloaded, checked and shipped unchanged with its licence.
- WinSparkle (Windows updates): MIT, downloaded, checked and shipped unchanged with its licence.
- Android's keyboard layouts (`android/helper/layouts/aosp`), from Android's InputDevices app: Apache 2.0. AAE's layouts are generated from them by `android/helper/layouts/generate.py`.
