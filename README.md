# Accessible Android Emulator (AAE)

AAE lets blind people create, run and test Android virtual devices without sighted help. Each device has a screen reader on from its first boot, takes your keyboard, and plays its audio reliably.

There's a Mac app, a Windows app, AAE Remote for Android phones, the `aae` command line, and an MCP server for AI agents, all over the same core. AAE is in early development: the Mac app is the most complete, and the Windows app is new and being tested.

## Getting AAE

- **Mac:** download the disk image from the [latest release](https://github.com/aaron-gh/accessible-android-emulator/releases/latest), open it and drag AAE to Applications. It needs a Mac with Apple silicon and macOS 13 or later. AAE isn't notarised by Apple, so the first time you open it, go to System Settings, Privacy & Security, and choose Open Anyway.
- **Windows:** download the installer, `AAE-<version>-windows-x64-setup.exe`, from the [latest release](https://github.com/aaron-gh/accessible-android-emulator/releases/latest) and run it. It installs AAE for you alone, with no administrator prompt. It needs Windows 10 or 11 on an Intel or AMD processor, with Windows Hypervisor Platform turned on. Google doesn't make its emulator for Windows on ARM, so AAE doesn't install there. AAE isn't signed for Windows yet, so SmartScreen may say it protected your PC: choose More info, then Run anyway.
- **Android phones:** AAE Remote uses the devices on your Mac or PC from your phone. Download `AAE-Remote-<version>.apk` from the latest release on the phone. See [AAE Remote](#aae-remote-for-android-phones) below.
- **Development builds:** every change to `master` is built for both and published as the [dev prerelease](https://github.com/aaron-gh/accessible-android-emulator/releases/tag/dev), replacing the one before, so it can be tried before a stable release. The latest are always at [AAE-dev.dmg](https://github.com/aaron-gh/accessible-android-emulator/releases/download/dev/AAE-dev.dmg) and [AAE-dev-windows-x64-setup.exe](https://github.com/aaron-gh/accessible-android-emulator/releases/download/dev/AAE-dev-windows-x64-setup.exe). Help, About AAE, says which build you have.

### System requirements

Android runs inside the emulator as a whole computer of its own, so AAE needs a reasonably capable machine. On less than this, Android starts, but it can be too slow to use with a screen reader: a laptop with an Intel N150, with four threads, was. AAE's setup and self-test say when a computer is below it.

- **Processor:** at least 6 threads, such as an Intel Core i5 or AMD Ryzen 5 from the last several years, or any Mac with Apple silicon. Low-power chips such as Intel's N100, N150 and N200, Celeron, Pentium Silver and Atom are too slow. Each device gets half the threads, up to 4.
- **Memory:** at least 8 GB; 16 GB to run more than one device at once.
- **Graphics:** devices draw their screens with the computer's graphics adapter. If the emulator can't use it, AAE starts the device again drawing in software, which is slower, and keeps it that way; the diagnostic report says which each device uses. Setting `AAE_GPU` to one of the emulator's `-gpu` modes, such as `swiftshader_indirect`, overrides this.
- **Disk:** an SSD, with about 10 GB free for the emulator, one Android version and a device, and around 2 GB more for each further Android version.
- **Mac:** Apple silicon and macOS 13 or later.
- **Windows:** Windows 10 or 11, 64-bit, on an Intel or AMD processor, with Windows Hypervisor Platform turned on.

Both apps update themselves. Check for Updates never offers a development build; from one, it offers the next stable release.

## What AAE does

### Setting up

- No Android Studio needed. On first run, AAE shows what it needs: Google's Android emulator, platform tools (adb) and build tools, about 500 MB. It shows Google's licence for you to accept, then downloads each one, checks it against Google's checksum, and installs it as Android Studio's SDK manager would. `aae setup` does the same on the command line.
- Before downloading, AAE checks the computer can run the emulator at full speed (Hypervisor.framework on a Mac, Windows Hypervisor Platform on Windows, KVM on Linux), and says what to do if it can't.
- AAE uses the Android SDK you already have, if there is one, such as Android Studio's. Otherwise it sets up its own, in its data folder. Set `ANDROID_HOME` to choose another place.
- AAE updates the emulator and platform tools it installed to Google's newest stable versions, with every device stopped, and mentions available updates when it starts, at most once a day. `aae setup --update` does it on the command line. Tools something else installed, such as Android Studio, are left for it to update; AAE marks the ones it installs with a `.installed-by-aae` file to tell them apart.

### Android versions

- `aae available` lists every Android version Google offers for this computer, Android 5 and later, including later updates to a version, such as Android 16's API 36.1. New Device in the apps, `aae create` and `aae download` install the one you choose into the SDK, the way Android Studio does. `--api` takes the version as `37`, `36.1`, or a preview's name.
- Previews of upcoming releases, such as betas and Google's Canary builds, are left out unless you ask for them: Include Previews in New Device, or `aae available --previews`. Google publishes previews, and some updates, only with 16 KB memory pages, for testing apps with them, and AAE says so beside them.
- Google's licence is never accepted for you. The apps show it with Accept and Decline: Escape declines, and Enter does nothing, so nothing is accepted by accident; the command line saves it for you to read, then you accept it with `--accept-licence`.
- The installed versions are listed with their size and the devices that use each one, and can be deleted to free disk space. A version can't be deleted while any of AAE's devices use it. Android Studio shares the SDK, so AAE also names any Android Studio devices that use the version, and warns that they won't start without it.

### Devices

- Create, copy, rename, wipe and delete. Wipe resets to first setup and keeps name, hardware and volume.
- Hardware (stopped devices): memory, cores, storage, screen size and density, from next start. `aae hardware <device> memory 4096 cores 6 storage 16G screen 1440x3120 density 560`; `cores auto` reverts to AAE's choice. Reducing storage needs a wipe.
- Export and Import: one `.aaedevice` file with apps, data, settings and named snapshots, between computers with the same processor architecture. The Android version must be installed on the target. `aae export`, `aae import`.
- No emulator window. Stop saves state for a quick start. Restart restarts Android only.
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

- Device mode gives your whole keyboard to Android, including the system's own shortcuts, until you press the way back. Your computer's screen reader keeps its own keys.
- A full PC keyboard in Android. The emulator's own keyboard layout is a phone layout with no Meta key, so TalkBack's keyboard shortcuts can't work, and Escape, Home and End act as phone buttons. AAE's helper supplies a full keyboard layout and selects it every time a device starts. Tested on Android 8, 11, 14 and 16.
- `aae keytest` checks that 15 kinds of key, Meta included, reach Android as the keys pressed. `aae latency` measures the time from a key press to the device's speech.
- `aae attach` plays the device's audio and sends your terminal's keyboard to the device, until Control-]. Plain Escape goes to the device. A terminal can't see the Command key, so Option is sent as Android's Meta key, the modifier TalkBack's current keymap uses; pass `--keep-alt` to send it as Alt. macOS Terminal turns Option-Left and Option-Right into word movement before AAE sees them. To fix that, open Terminal's Settings, then Profiles, then Keyboard. Turn on "Use Option as Meta key". Set Option-Left to send `\033[1;3D` and Option-Right to send `\033[1;3C`. iTerm2, Ghostty, kitty and WezTerm report every key and need none of this. The apps capture keys directly, with none of these limits.

### Gestures

- Screen reader gestures, performed with simulated fingers through the emulator's touchscreen, so the screen reader sees them as real touches: swipes in four directions, two-part swipes such as up then left, swipes with two, three or four fingers, single, double and triple taps, and double tap and hold. They follow the way the screen is turned. Tested with Backtalk on Android 8, 14 and 16.
- Gesture mode turns the whole keyboard into gestures:
  - Arrows swipe; hold one arrow and press another for a two-part swipe.
  - Space double tap, T tap, R triple tap, H double tap and hold, L touch and hold.
  - Hold 2, 3 or 4 for that many fingers.
  - Touch point: Tab and Shift-Tab move between screen elements, Shift-arrows move in steps, C centres, W reports the position. Works without a screen reader running.
  - Question mark lists the keys.
- `aae gesture <device> swipe-right double-tap`; `--at X,Y`. `aae inspect <device> --targets` lists touch targets.

### Battery, Location, Phone and Network

- Battery level, charging and health (good, failed, dead, over voltage, overheated).
- Fingerprint touches, fingers 1 to 10. Enroll them in Android's security settings.
- Shake.
- Location from coordinates, a place or an address (looked up on OpenStreetMap; only the typed text is sent).
- Play GPX Route: moves the location once a second along the file's track, route or waypoints, at the file's pace or 30 km/h, with a speed multiplier. Choose again to stop.
- Text messages to the device; incoming calls, and answer, busy or hang up for outgoing calls.
- Network: airplane mode, Wi-Fi, mobile data, and speed (full, LTE, 3G, slow 3G, EDGE, GPRS).

CLI: `battery`, `fingerprint`, `shake`, `location`, `route`, `sms`, `call`, `network`.

### Display and Language

Language, font size, display size, dark theme (Android 10+), bold text (Android 12+), high contrast text, colour inversion, colour correction, animations, captions and touch and hold delay. Languages include the pseudo-locales en-XA (accented, longer) and ar-XB (right to left), and any typed language tag. Use These Settings for New Devices applies them to devices created later. `aae settings <device> [name value]...`, `--list`, `--for-new-devices`, `--forget-new-devices`.

### Testing tools

The inspector, checks and speech log were checked on Android 8, 11, 14 and 16.

- Accessibility Inspector (`aae inspect`): the accessibility tree as a screen reader describes it, with all properties. Tree or list; copy as text; save as HTML, JSON or text (`--html`). Follow the screen (`--follow`) refreshes when the screen changes.
- Accessibility checks (`aae check`): unlabelled controls, undescribed images, small touch targets, duplicate labels.
- Speech Log (`aae speech-log`): screen reader utterances with times. Adds about 10 ms.
- Device Log (`aae logs`): filter by app, tag, level and text; announce new errors; pause, copy, save. `--follow`.
- Shell (`aae shell`): runs a command as the shell user. Two-minute limit.
- Record Screen (`aae record`): screen and audio to WebM, up to three minutes.

## The Mac app

The Mac app is in `macos/`. Announcements go through VoiceOver when it's running, and otherwise through the Mac's system voice, each after a short sound, which Settings can turn off. AAE menu, Check for Updates, checks for a new version straight away; it also checks by itself.

- File: New Device (Command-N), Import Device, Android Versions (Option-Command-A), Serve Devices to Phones.
- Device:
  - Start (Command-Shift-S), Stop (Command-Shift-.), Restart (Command-Shift-R), Use Android Keyboard (Command-Shift-E), Use Gestures (Command-Shift-G), Open in Own Window (Option-Command-O), Speak Status (Command-Shift-I).
  - Android buttons including Power and Assistant, rotation, Hardware, Export, rename, copy, wipe, delete.
  - Accessibility Inspector (Option-Command-I), Speech Log (Option-Command-L), Device Log (Option-Command-J), Shell (Option-Command-T), Apps (Option-Command-P), Accessibility Services (Option-Command-U), Snapshots (Option-Command-S), Battery, Location, Phone and Network (Option-Command-B), Display and Language (Option-Command-Comma).
  - Mute (Command-Shift-M), volume (Option-Command-Up and Down), Check Audio (Option-Command-K), Turn On Microphone (Command-Shift-U), Turn On Speech Bridge.
  - Clipboard: device to Mac (Command-Shift-C), Mac to device (Command-Shift-V), type the Mac clipboard (Option-Command-V).
  - Record Screen (Option-Command-R), Save Screenshot.
  - Install App (Command-I), or drop or paste APKs. Install Screen Reader Build (Option-Shift-Command-I). Open Link (Command-Shift-L).
- Main window: volume and Audio output.
- Device mode: Command is Meta. Spotlight, Mission Control and other system shortcuts go to Android; VoiceOver shortcuts still work. Control-Command-Escape or the Return to the Mac button exits. Settings offers Control-Shift-Command-Escape or Control-Option-Command-Escape instead.
- Open in Own Window: the device whose window is in front is the selected device.

## The Windows app

The Windows app is in `crates/aae-windows`. It's written in Rust with Windows' own standard controls, which screen readers know best, and it's built on the Mac. It installs for you alone, with no administrator prompt, and updates itself: Help, Check for Updates.

Announcements go straight to NVDA, through NV Access's controller client, which also shows them in braille, or to JAWS, through its own speech interface. Narrator and other screen readers get them as UI Automation notifications, and with no screen reader, Windows' own voice speaks them. A short sound plays before each one.

- Mac shortcuts with Control for Command and Alt for Option. Rename is F2, Delete is Delete.
- Device mode: the Windows key is Meta. Alt-Tab, the Windows key and Alt-F4 go to Android; Control-Alt-Delete and Windows-L can't. Keys held with Insert or Caps Lock go to the host screen reader. Control-Windows-Escape or the Return to Windows button exits; Settings offers Control-Shift-Windows-Escape or Control-Alt-Windows-Escape.
- Device, Audio Output; `aae audio-output <device> <name>`. Falls back to the system default when the output is missing.
- Tool windows as on the Mac: Tab moves, F5 refreshes, Escape closes.
- In a device's own window, menu shortcuts act on that device; device and gesture mode run in the main window.

## AAE Remote, for Android phones

AAE Remote uses the devices on a Mac or PC running AAE from an Android phone. Google's emulator can't run on a phone, so the devices run on the computer, and the phone is their remote: their sound and vibrations play on the phone, and the phone sends them keys and touches. It needs Android 8 or later, and gesture mode needs Android 11. Its code is in `android/remote`, and the computer's side is `crates/aae-remote`.

- **Install:** `AAE-Remote-<version>.apk` from the latest release; `AAE-Remote-dev.apk` in the dev prerelease. If Play Protect blocks it: More details, Install anyway.
- **Updates:** checked daily and with Check for Updates, verified against AAE's update feed, confirmed by Android. Offer development builds opts in.
- **Serving:** File, Serve Devices to Phones, or `aae serve` (no display or audio device needed). Allow incoming connections if asked.
- **At login:** Serve at login, without AAE open (in Serve Devices to Phones) or `aae daemon install|uninstall|status`: a LaunchAgent, a Windows Run entry or a systemd user service. `aae pair` makes a pairing code for it. Logs to `serve.log`.
- **Pairing:** Pair with a Computer: choose a computer on the network or type an address, then enter its 12-character code (single use, 10 minutes). TLS with the certificate pinned. `aae phones`, `aae phones --unpair <id>`.
- **Managing:** devices (create, start, stop, restart, wipe, copy, rename, delete), Android versions (download after accepting the licence on the phone, delete), tool setup and updates.
- **Device screen:** Android buttons, Speak Status, rotation. Testing Tools: speech log, device log, inspector, apps, services, snapshots, Battery, Location, Phone and Network (including GPX upload), Display and Language, links and intents, clipboard, APK install.
- **Microphone:** sends the phone's microphone while an app on the device records (voice communication source, echo cancelled). Foreground only. Testing Tools uploads an audio file to play into it.
- **Speech bridge:** sends the device's speech to the phone's text-to-speech. Engine and rate: Bridge Text-to-Speech Settings (default: system). Takes priority over a desktop app while the phone is attached.
- **Keyboard mode:** a connected keyboard's keys go to the device. Control-Shift-Escape or a long press of volume down exits.
- **Gesture mode:** the phone's screen is the device's touchscreen, multi-finger gestures included. Long press of volume down exits; volume keys change the device's volume. Needs the AAE gesture mode accessibility service (Samsung: under Installed apps), which passes touches through only while gesture mode is open. Without it, buttons perform common gestures.
- **Restricted setting (Android 13+):** after one attempt to turn the service on: Settings, Apps, AAE Remote, More options, Allow restricted settings.

## The command line

The `aae` command comes with both apps: on the Mac it's inside the app, at `/Applications/AAE.app/Contents/Helpers/aae`, and on Windows it's next to the app, at `%LOCALAPPDATA%\Programs\AAE\aae.exe`. `aae help` lists every command, and `aae help <command>` explains one. `aae serve` serves this computer's devices to AAE Remote. Commands that act on a device take its name. For example:

```sh
aae setup
aae available
aae create "Android 16 test" --api 36 --backtalk
aae attach "Android 16 test"
aae stop "Android 16 test"
```

- Setting up: `setup`, `doctor`, `self-test`, `report`.
- Android versions: `available`, `download`, `images`, `remove-image`.
- Devices: `list`, `create`, `clone`, `rename`, `hardware`, `export`, `import`, `wipe`, `delete`, `start`, `restart`, `stop`, `status`, `snapshot`.
- Keyboard, audio and speech: `attach`, `listen`, `playback-volume`, `audio-output`, `volume`, `key`, `type`, `gesture`, `keytest`, `latency`, `sound-check`, `audio-check`, `mic`, `speech`, `speech-bridge`.
- Screen readers and apps: `screen-reader`, `services`, `install`, `apps`, `app`, `watch`, `link`, `intent`.
- Testing: `inspect`, `check`, `speech-log`, `logs`, `shell`, `screenshot`, `record`.
- Conditions: `rotate`, `battery`, `fingerprint`, `shake`, `location`, `route`, `network`, `settings`, `sms`, `call`, `clipboard`.
- AAE Remote: `serve`, `pair`, `phones`, `daemon`.
- AI agents: `mcp`.

## AI agents (MCP)

`aae mcp` is an MCP server, so AI agents can run, use and inspect AAE's devices, as a tester would. It works on any device AAE runs, through the same core as the apps, so you can watch or listen in the app while an agent works.

Besides the screen, an agent can hear what a blind user hears. The speech log records what the screen reader says, so an agent can swipe through an app with the screen reader's own gestures and check what was announced, not just what was drawn.

- Observe: `screenshot`, `inspect`, `check_accessibility`, `touch_targets`, `speech_log`, `device_log`.
- Act: `gesture`, `tap_target`, `activate_target`, `type_text`, `press_key`, `open_app`, `open_link`, `send_intent`, notifications, quick settings, `rotate`, `device_settings`, battery, `touch_fingerprint`, `shake`, location, `play_route`, `stop_route`, network, SMS, calls, clipboard, `play_into_microphone`, `start_recording`, `stop_recording`.
- Wait: `wait_for` text on screen or in speech, or its absence; `pause`.
- Set up: devices, `install_app`, apps, permissions, accessibility services, snapshots. No Android version downloads (the user accepts Google's licence in the app).
- Wipe, delete, delete snapshot, uninstall, clear data and shell need `aae mcp --allow-destructive`. Loading a snapshot is marked destructive.

```sh
claude mcp add aae -- /Applications/AAE.app/Contents/Helpers/aae mcp
```

On Windows, in PowerShell:

```sh
claude mcp add aae -- "$env:LOCALAPPDATA\Programs\AAE\aae.exe" mcp
```

Other agents take the same command in their settings, such as `{"mcpServers": {"aae": {"command": "/Applications/AAE.app/Contents/Helpers/aae", "args": ["mcp"]}}}`. Add `--allow-destructive` after `mcp` for the tools that can't be undone.

## Where things are kept

Devices live in AAE's data folder, under `devices`. On macOS that's `~/Library/Application Support/io.github.aaron-gh.AAE`, and on Windows `%LOCALAPPDATA%\aaron-gh\AAE\data`. Set `AAE_HOME` to keep them somewhere else, such as an external drive. Each device is an ordinary emulator AVD folder plus an `aae.toml` file. To use AAE's devices from Android Studio, point `ANDROID_AVD_HOME` at the folder.

Android versions are installed into the Android SDK, where Android Studio sees them too.

To set a default screen reader, so you don't have to choose one each time, put its APK in the data folder as `screen-readers/default.apk`, or set `AAE_SCREEN_READER_APK` to its path.

## Diagnosing problems

- Help, Run Self-Test in either app, or `aae self-test`, checks everything AAE needs and reads out what it found: the computer's virtualisation and sound output, the SDK, AAE's own parts, free disk space, and for each running device, its screen reader, AAE's helper and its speech. The apps also check they can capture the keyboard, and that each open device's sound is getting through. It makes no sound.
- To report a bug, attach a diagnostic report: Help, Save Diagnostic Report in either app, or `aae report`. It's plain text: AAE's version, this computer, the SDK, your devices and the end of their emulator logs, and AAE's own log. Your home folder and computer name, and on the Mac your full name, are taken out, and it never includes what you typed on a device, your clipboard, or the device's own log.
- AAE keeps its log in its data folder, under `logs`, at most about 4 megabytes. It records AAE's steps and problems, never keys, typed text or clipboards.
- `AAE_LOG=info aae start "My device"` shows each step of starting a device. Use `debug` for more.
- `AAE_KEYLOG=1` records every key the Mac app captures. Key codes reveal what you type, so leave it off otherwise.
- `crates/aae-core/examples/audio_probe.rs` prints how loud the device's audio stream is, in quarter seconds, while pressing keys.

## Building

You need Rust (from [rustup.rs](https://rustup.rs)). You don't need protoc or CMake.

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

The Windows app and the Windows `aae` command are built on the Mac too, cross-compiled with MinGW-w64:

```sh
brew install mingw-w64 makensis
windows/build.sh
```

That adds Rust's Windows target if it's missing, builds `AccessibleAndroidEmulator.exe` and `aae.exe`, and puts them with the helper app, eSpeak NG, NVDA's controller client, WinSparkle and a read-me into an installer and a zip in `windows/dist`. Both programs carry everything they need, so Windows needs no other files to run them.

eSpeak NG is built from its source, a git submodule in `android/third_party/espeak-ng`, by `android/build-espeak.sh`. The first build takes a few minutes and downloads the Android NDK version it needs. Clone AAE with `--recurse-submodules`, or the script fetches the submodule itself. eSpeak NG is under GPL v3.

The helper and eSpeak NG are signed with AAE's own Android key, at `~/.android/aae.keystore`, with its password in `AAE_ANDROID_KEY_PASSWORD` or, on a Mac, the Keychain item "AAE Android signing key". Without it, they're signed with your computer's debug key. Either way works: when a device has a copy signed with another key, AAE reinstalls it.

AAE finds the built helper and eSpeak NG on its own when you run it from this folder. Alternatively, put them next to the `aae` program as `aae-helper.apk` and `aae-espeak.apk`, or set `AAE_HELPER_APK` and `AAE_ESPEAK_APK` to their paths.

Development builds are made by `.github/workflows/dev-build.yml`, with the same scripts.

## Layout

- `crates/aae-core`: the cross-platform core. It covers the SDK, downloading Android versions, the device store, the emulator, gRPC control, adb, audio, keys, provisioning, the accessibility inspector and the device log.
- `crates/aae-cli`: the `aae` command.
- `crates/aae-mcp`: the MCP server, `aae mcp`.
- `crates/aae-remote`: the server for AAE Remote, `aae serve`: pairing, the devices' sound and vibrations, keys, touches and the testing tools, over TLS and WebSocket.
- `crates/aae-ffi`: the bridge from the core to the apps, generated with UniFFI for the Mac app and used directly by the Windows app.
- `macos`: the Mac app, in Swift. `appcast.xml` is its update feed.
- `crates/aae-windows`: the Windows app. `windows` has its build and release scripts, the installer script, and the read-me that goes with it. `appcast-windows.xml` is its update feed.
- `crates/aae-core/proto`: the Android Emulator's gRPC definitions, under Apache 2.0.
- `android/helper`: AAE's helper app. It is an accessibility service, because Android lets only accessibility services set the accessibility volume and read the screen for the inspector. It also carries the full keyboard layout, a small tool AAE runs as the shell user to select it and to watch the vibrator for AAE Remote, the speech check, the speech log's relay engine, and the silent test tone used to measure audio speed.
- `android/remote`: AAE Remote, the Android app, in Kotlin with Android's standard controls, and its gesture mode accessibility service. `android/build-remote.sh` builds it.
- `android/espeak`: how AAE builds eSpeak NG: its own app ID, signed like the helper, with eSpeak NG's code unchanged.
- `android/third_party/espeak-ng`: eSpeak NG's source, as a git submodule.

## Licence

AAE is under the [Apache License 2.0](LICENSE). Parts it includes keep their own licences:

- The Android Emulator's gRPC definitions, in `crates/aae-core/proto`, are under Apache 2.0.
- eSpeak NG, in `android/third_party/espeak-ng`, is under the GNU General Public License version 3. AAE builds it unchanged as a separate app that runs on the Android device.
- Sparkle, which updates the Mac app, is under the MIT licence.
- NV Access's NVDA controller client, `nvdaControllerClient.dll`, which the Windows app speaks through, is under the GNU Lesser General Public License version 2.1. `windows/build.sh` downloads it from NV Access, checks it, and ships it unchanged with its licence.
- WinSparkle, which updates the Windows app, is under the MIT licence. `windows/build.sh` downloads it, checks it, and ships it unchanged with its licence.
