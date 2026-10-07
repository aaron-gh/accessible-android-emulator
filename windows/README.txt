Accessible Android Emulator (AAE) for Windows: test build

AAE creates, runs and tests Android virtual devices without sighted help. This is the first Windows build. It is for testing, and parts of the Mac app are still to come; see "Not in the Windows app yet" below.

WHAT YOU NEED

- Windows 10 or 11, 64-bit, on an Intel or AMD processor. Google's Android emulator isn't made for Windows on ARM, such as Snapdragon laptops, so AAE doesn't install there.
- Windows Hypervisor Platform turned on, which the Android emulator needs to run at full speed. In Control Panel, choose Programs, then "Turn Windows features on or off", check "Windows Hypervisor Platform", and restart. Virtualisation must also be on in the computer's firmware settings; it usually is. The self-test, in the Help menu, says whether the emulator can use it.
- About 10 gigabytes free for the emulator, one Android version and a device.

STARTING

Run the installer, AAE-...-setup.exe. It installs AAE for you alone, in your own Programs folder, with no administrator prompt, and adds Accessible Android Emulator to the Start menu. Uninstalling it, from Installed apps in Settings, leaves your devices and Android versions in place.

AAE isn't signed yet, so Windows SmartScreen may say it protected your PC. Choose "More info", then "Run anyway".

AAE updates itself. It asks once whether to check automatically, and Check for Updates, in the Help menu, checks straight away. When there's a new version, it downloads and installs it, and starts AAE again. Development builds are never offered as updates; if you have one, you're offered the next stable release.

The files next to AAE are part of it: aae-helper.apk and aae-espeak.apk go onto your devices, nvdaControllerClient.dll is how AAE speaks through NVDA, and WinSparkle.dll is how it updates.

The first time, AAE downloads Google's Android emulator and tools, after saying how big they are and showing you Google's licence to accept. Then choose New Device in the File menu (Control N). AAE downloads the Android version you choose, again after its licence, creates the device and starts it with a screen reader.

AAE speaks its messages through your screen reader: straight through NVDA or JAWS, which also shows them on a braille display with NVDA, and through Narrator and others as accessibility notifications. When no screen reader is running, Windows' own voice speaks them. The device's own speech plays through your computer's sound.

THE KEYBOARD

Control Shift E gives the keyboard to Android, so every key goes to the device: the Windows key is Meta, the key TalkBack and Backtalk use for their shortcuts, and Alt Tab, Alt F4 and the like go to Android too. Control Windows Escape gives the keyboard back to Windows.

Your screen reader keeps its own keys: while Insert or Caps Lock is held, keys go to Windows, so NVDA and JAWS commands still work. If you click another window, Windows gets the keyboard back until AAE is in front again.

Control Shift G starts gesture mode, where keys perform touch screen gestures. Arrows swipe. Hold one arrow and press another for a two-part swipe, such as up then left. Space double taps, T taps, R triple taps. H double taps and holds, and L touches and holds, until you let go. Hold 2, 3 or 4 while pressing a key to use that many fingers. Tab and Shift Tab move the touch point from item to item, Shift arrows move it a step, C puts it in the middle, W says where it is, and question mark reads these keys out. Control Windows Escape returns to Windows.

The Device menu has everything else, with its shortcuts: Back (Control Shift B), Home (Control Shift H), notifications, rotating, the device's volume, its clipboard, installing apps, screenshots, renaming, copying, wiping and deleting devices. Control Shift I speaks the status. To install apps, choose Install App (Control I), or copy them in File Explorer and paste them into AAE with Control V.

NOT IN THE WINDOWS APP YET

The accessibility inspector, speech log, device log, shell, apps, accessibility services, snapshots, battery and location, and the Android versions window. Until they are, the aae command in AAE's folder does all of them: open a command prompt there and run "aae help". For example, "aae inspect Pixel", "aae speech-log Pixel" or "aae services Pixel".

REPORTING PROBLEMS

Choose Save Diagnostic Report in the Help menu. It saves a text file with AAE's log and details of your computer and devices, with your home folder and computer name taken out, which you can read before sending. Please say which screen reader you use, and whether AAE's messages were spoken.

LICENCES

AAE is under the Apache License 2.0, in LICENSE.txt. nvdaControllerClient.dll is NV Access's NVDA controller client, unchanged, under the GNU Lesser General Public License 2.1, in nvdaControllerClient-LICENSE.txt; its source is part of NVDA, at https://github.com/nvaccess/nvda. WinSparkle.dll is WinSparkle, by Vaclav Slavik, unchanged, under the MIT licence, in WinSparkle-LICENSE.txt.

https://github.com/aaron-gh/accessible-android-emulator
