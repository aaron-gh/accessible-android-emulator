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

- Named devices, including several of the same Android version, each with its own apps and data: create, copy, rename, wipe and delete. Wiping takes a device back to how it was first set up: its apps, data and snapshots go, and its screen reader is set up again, but it keeps its name, hardware and volume.
- Devices run with no emulator window. Stopping one saves its state for a quick start next time, after first writing everything to its disk. Restarting restarts Android without losing anything on it.
- First-boot setup: hardware keyboard on, setup wizard skipped, screen kept awake, animations off, and a screen reader installed and turned on.
- Choosing a screen reader. If the Android image has no screen reader and you didn't choose one, AAE asks whether to download Backtalk (its latest development build, checked against Backtalk's signing key), install your own APK, or go without.
- Screen reader builds, for screen reader developers: a new build can go on several devices at once, starting with the ones that use that screen reader. A new build of the same screen reader keeps its settings, and stopped devices get it when they next start. A build signed differently can only replace the old one, which loses its settings, so AAE asks first.
- Accessibility services stay as you set them. Every start, and every app install, turns back on any service Android turned off. The Accessibility Services window lists every service on a device by its name and description: screen readers are one choice, as only one runs at a time, so switching between TalkBack, Backtalk or any other is one step, and every other service has a switch.
- Installing apps, on one device or several at once. The first time an app has an accessibility service, a keyboard, a notification listener or a device administrator, AAE asks whether to turn each one on, and remembers the answer for that device, so a reinstall or update applies it again without asking. Watching an app's builds installs each new build as soon as it's written, from an APK or a build folder such as build/outputs/apk.
- Managing apps: a device's apps are listed by the names people see, and can be opened, force-stopped, cleared or uninstalled. Each permission an app asks for is a switch, with Grant All, as is its special access: unrestricted battery use, display over other apps, usage access and modifying system settings.
- Opening links, in an app you choose or whichever Android picks, and sending any intent, to open a screen or as a broadcast, with text extras.
- Sending keys and text, rotating, the clipboard both ways, screenshots, and named snapshots with notes. Battery level and charging, location from an address, a place name or coordinates, text messages to the device, and the other end of a phone call: call the device, hang up, hold, or answer or be busy for a call the device makes.

### Speech and sound

- The device's audio plays through AAE's own audio code, not the emulator's, whose output is off, which removes its crackle. Each device has its own volume, remembered, and can be muted. With several devices running, AAE plays only the one you're using, unless you turn that off in Settings.
- Each new device's screen reader volume is turned up to full, through AAE's helper app. Pass `--no-volume-boost` to leave it alone, or use `aae volume` to change it.
- Dependable speech. During setup and on every start, AAE's helper checks that the device's speech engine can actually speak. If Google's engine has downloaded voices that don't work, AAE resets it. If speech still fails, or the image has no engine, AAE installs its own build of eSpeak NG and makes it the default. That build speaks as soon as it's installed. `aae speech` runs the check by hand.
- Checking the sound. The apps watch that each device's sound is reaching them: when Android plays something and nothing arrives, they say so, once. Check Audio also has AAE's helper play a test tone, which AAE listens for with its own playback muted so you don't hear it, and if the sound isn't getting through, offers to restart AAE's audio without restarting the device. `aae sound-check` does the same test in the terminal.
- The microphone. Turn On Microphone, in the Device menu, sends your computer's microphone into the selected device, for voice typing, voice search or calls, until you turn it off or the device stops. The device hears it whenever an app on it listens: AAE's helper says when Android starts recording, and the sound goes in from half a second later, since the emulator crashes if sound arrives sooner. The emulator itself never opens your microphone: AAE does, and only while it's on. Check Microphone plays a tone into the device's microphone and has the device listen for it, with no sound on the computer. Play Audio File into Microphone plays a WAV, MP3, FLAC, Ogg Vorbis or M4A file into the device the next time an app on it listens, from the start, for testing voice typing and search with the same words every time. `aae mic <device> on`, `check`, `level` and `play <file>` do the same in the terminal.
- Correct pitch on older Android. In the emulator, Android 10 and earlier play audio about 8% slow, so they sound low. AAE measures each device's audio speed silently during setup, and raises the pitch back when playing. Turn this off in Settings, or with `--no-pitch-correction`. `aae audio-check` measures again.

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

### Display and language

Display and Language (Option-Command-Comma on the Mac, Control-Alt-Comma on Windows) changes the device's language, font size, display size, dark theme, bold text, high contrast text, colour inversion, colour correction, animations, captions and touch and hold delay, without going through Android's Settings. The languages include two pseudo-locales, accented and longer text, and right to left, for finding text that's cut off or laid out the wrong way; any other language tags can be typed. `aae settings <device>` says them all, `aae settings <device> font-size 150 language ar-XB` changes them, and `aae settings --list` lists the choices. AI agents have `device_settings`, and AAE Remote has Display and Language in Testing Tools. Dark theme needs Android 10, and bold text Android 12.

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

- File menu: New Device (Command-N) and Android Versions (Option-Command-A).
- Device menu, for the selected device: Start (Command-Shift-S), Stop (Command-Shift-.), Restart (Command-Shift-R), Use Android Keyboard (Command-Shift-E), Use Gestures (Command-Shift-G), Open in Own Window (Option-Command-O) and Speak Status (Command-Shift-I). Then Android's buttons and screens, including Power, Assistant and the device's own volume buttons, rotation, the device's sound, the clipboard, apps and services, conditions, installing, and renaming, copying, wiping and deleting.
- Testing tools: Accessibility Inspector (Option-Command-I), Speech Log (Option-Command-L), Device Log (Option-Command-J) and Shell (Option-Command-T).
- Apps (Option-Command-P), Accessibility Services (Option-Command-U), Snapshots (Option-Command-S), Battery, Location, Phone and Network (Option-Command-B), and Display and Language (Option-Command-Comma).
- Sound: Mute (Command-Shift-M), Turn Device Audio Up and Down (Option-Command-Up and Down), Check Audio (Option-Command-K), Turn On Microphone (Command-Shift-U), and a volume slider in the main window. The first time, macOS asks whether AAE may use the microphone.
- Clipboard: Command-Shift-C copies the device's clipboard to the Mac, Command-Shift-V sends the Mac's clipboard to the device, and Option-Command-V types it on the device as key presses, for fields that block pasting.
- Recording: Record Screen (Option-Command-R) records the screen and its sound into a WebM file, for up to three minutes, the emulator's limit; choose it again, Stop Recording, to stop. On Windows it's Control-Alt-R, and in the terminal `aae record <device> <file>`. AI agents have `start_recording` and `stop_recording`.
- Installing: Install App (Command-I), or drop APKs on the window, or copy them in Finder and paste them into it. Install Screen Reader Build is Option-Shift-Command-I, and Open Link is Command-Shift-L.
- Device mode sends every key to Android, with Command as Meta, including system shortcuts such as Spotlight's Command-Space and Mission Control. VoiceOver's own shortcuts, such as Command-F5, still work. Control-Command-Escape, or the "Return to the Mac" button, brings the keyboard back.
- Several devices at once: Open in Own Window gives a device a window of its own. The device whose window is in front is the selected one, so the Device menu acts on it, and device or gesture mode started there stays in that window.

## The Windows app

The Windows app is in `crates/aae-windows`. It's written in Rust with Windows' own standard controls, which screen readers know best, and it's built on the Mac. It installs for you alone, with no administrator prompt, and updates itself: Help, Check for Updates.

Announcements go straight to NVDA, through NV Access's controller client, which also shows them in braille, or to JAWS, through its own speech interface. Narrator and other screen readers get them as UI Automation notifications, and with no screen reader, Windows' own voice speaks them. A short sound plays before each one.

- Shortcuts are the Mac app's, with Control for Command and Alt for Option: New Device is Control-N, Start is Control-Shift-S, and so on. Rename is F2, and Delete is the Delete key.
- Device mode sends every key to Android, with the Windows key as Meta, including Windows' own shortcuts such as Alt-Tab, the Windows key and Alt-F4; only Control-Alt-Delete and Windows-L can't be taken. Your screen reader keeps its keys: while Insert or Caps Lock is held, keys go to Windows. Control-Windows-Escape, or the "Return to Windows" button, brings the keyboard back.
- Turn On Microphone is Control-Shift-U.
- Installing apps: Install App (Control-I), or drop APKs on the window, or copy them in File Explorer and paste them into it with Control-V.
- It has the Mac app's windows, with the same shortcuts: Accessibility Inspector (Control-Alt-I), Speech Log (Control-Alt-L), Device Log (Control-Alt-J), Shell (Control-Alt-T), Apps (Control-Alt-P), Accessibility Services (Control-Alt-U), Snapshots (Control-Alt-S), Battery, Location, Phone and Network (Control-Alt-B), Display and Language (Control-Alt-Comma), Android Versions (Control-Alt-A), Open Link (Control-Shift-L), Send Intent, Watch for New Builds, and Open in Own Window (Control-Alt-O). Each window stays open beside the main one: Tab moves through it, F5 refreshes it if it has a Refresh button, and Escape closes it.
- In a device's own window, the menu's shortcuts act on that device. Device and gesture mode started there take the keyboard in the main window, which has the way back.
- Battery, Location, Phone and Network looks up places and addresses on OpenStreetMap, which gets only the text you typed; latitude and longitude are used as they are.

## AAE Remote, for Android phones

AAE Remote uses the devices on a Mac or PC running AAE from an Android phone. Google's emulator can't run on a phone, so the devices run on the computer, and the phone is their remote: their sound and vibrations play on the phone, and the phone sends them keys and touches. It needs Android 8 or later, and gesture mode needs Android 11. Its code is in `android/remote`, and the computer's side is `crates/aae-remote`.

- **Getting it:** on the phone, download `AAE-Remote-<version>.apk` from the [latest release](https://github.com/aaron-gh/accessible-android-emulator/releases/latest), and install it. Android may ask you to allow installing apps from your browser. Development builds are `AAE-Remote-dev.apk` in the [dev prerelease](https://github.com/aaron-gh/accessible-android-emulator/releases/tag/dev).
- **Updates:** AAE Remote checks for a new version once a day, and Check for Updates checks straight away. It downloads the update, checks it against AAE's update feed, and hands it to Android, which asks you to confirm. The first time, Android asks you to allow AAE Remote to install apps. It offers stable releases, and development builds too if you turn on "Offer development builds"; from a development build, it offers the next stable release. Google Play Protect may block the APK, first install or update, as from a developer it hasn't seen: choose More details, then Install anyway.
- **Serving:** on the computer, choose File, Serve Devices to Phones, and turn serving on. Or run `aae serve`, which needs no screen, speakers or desktop app: a computer in a cupboard can run devices for phones, with all their sound going to the phone. The first time, macOS or Windows may ask whether to accept incoming network connections: allow it.
- **Serving at login:** in Serve Devices to Phones, turn on "Keep serving whenever I log in, without AAE open", or run `aae daemon install`. AAE then serves from the moment you log in, whether or not it's open: a LaunchAgent on the Mac, an entry in Windows' Run list, or a systemd user service on Linux. `aae daemon uninstall` stops it, and `aae daemon status` says whether it's set up and serving. `aae pair` makes a pairing code for it, as does New Pairing Code in the apps. It writes to `serve.log` in AAE's logs folder.
- **Pairing:** in AAE Remote, choose Pair with a Computer. Computers serving on the same network are listed; for others, type the address. Then enter the twelve-character code the computer shows and speaks. Each code works once, for ten minutes. The connection is encrypted, and the phone remembers the computer's certificate, so it won't connect to anything pretending to be it. `aae phones` lists the paired phones, and `aae phones --unpair <id>` removes one.
- **Managing:** list, create, start, stop, restart, wipe, copy, rename and delete devices; download Android versions, after you read and accept Google's licence on the phone, and delete them; set up and update the emulator and tools.
- **Using a device:** while its screen is open, its sound plays on the phone, and its vibrations play on the phone's vibrator, each as long as it was on the device. Android's buttons, Speak Status and rotation are buttons. Testing Tools has the speech log, device log, accessibility inspector, apps, accessibility services, snapshots, battery, location, phone and network, links and intents, the clipboard, and installing an APK from the phone.
- **Microphone:** Turn On Microphone, on a device's screen, sends the phone's microphone into the device whenever an app on it listens, for voice typing or calls. It records as for a phone call, so the device's own sound from the phone's speaker is cancelled out. It works while AAE Remote is on screen. Testing Tools can also send a sound file from the phone, to play into the device's microphone the next time an app on it listens.
- **Keyboard mode:** with a keyboard connected to the phone, every key goes to the device. Control-Shift-Escape, or a long press of volume down, comes back.
- **Gesture mode:** the phone's screen becomes the device's touchscreen, so the device's screen reader gets your gestures, with every finger. A long press of volume down comes back, and volume up and down change the device's volume. Your phone's own screen reader would take the touches, so AAE Remote has an accessibility service, AAE gesture mode, that lets them through while gesture mode is open, and nowhere else. It works with TalkBack, Backtalk and other screen readers. Turn it on in Android's accessibility settings; on Samsung phones it's under Installed apps. In keyboard mode, it also takes keys before your phone's screen reader does.
- **"Restricted setting":** Android 13 and later block accessibility services in apps installed from a downloaded APK. Try to turn the service on once, then open Settings, Apps, AAE Remote, the More options menu, and choose Allow restricted settings. Then turn it on again. Without the service, gesture mode has buttons for common gestures instead.

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
- Devices: `list`, `create`, `clone`, `rename`, `wipe`, `delete`, `start`, `restart`, `stop`, `status`, `snapshot`.
- Keyboard, sound and speech: `attach`, `listen`, `playback-volume`, `volume` (the screen reader's own volume), `key`, `type`, `gesture`, `keytest`, `latency`, `sound-check`, `audio-check`, `mic`, `speech`.
- Screen readers and apps: `screen-reader`, `services`, `install`, `apps`, `app`, `watch`, `link`, `intent`.
- Testing: `inspect`, `check`, `speech-log`, `logs`, `shell`, `screenshot`, `record`.
- Device conditions: `rotate`, `battery`, `fingerprint`, `shake`, `location`, `network`, `settings`, `sms`, `call`, `clipboard`.
- AI agents: `mcp`, below.

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
