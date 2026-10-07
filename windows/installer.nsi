; AAE's Windows installer, built on the Mac with NSIS by windows/build.sh:
;   makensis -DVERSION=0.3.0 -DBUILD=40 -DSTAGE=<folder of files> -DOUTFILE=<setup.exe> windows/installer.nsi
;
; It installs for the current user, in their own Programs folder, so it needs
; no administrator rights, and adds AAE to the Start menu and to Installed
; apps. Updates run it with /S: it installs quietly and starts AAE again.
; Devices and settings are kept in AAE's data folder, never touched here.

Unicode true
SetCompressor /SOLID lzma
RequestExecutionLevel user
ManifestDPIAware true

!define NAME "Accessible Android Emulator"
!define EXE "AccessibleAndroidEmulator.exe"
!define UNINSTALL_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\AAE"

Name "${NAME}"
OutFile "${OUTFILE}"
InstallDir "$LOCALAPPDATA\Programs\AAE"
InstallDirRegKey HKCU "${UNINSTALL_KEY}" "InstallLocation"
BrandingText "AAE ${VERSION}"

VIProductVersion "${VERSION_NUMBERS}.${BUILD}"
VIAddVersionKey "ProductName" "${NAME}"
VIAddVersionKey "ProductVersion" "${VERSION}"
VIAddVersionKey "FileDescription" "${NAME} installer"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "Apache License 2.0"

!include "MUI2.nsh"

!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\${EXE}"
!define MUI_FINISHPAGE_RUN_TEXT "Start ${NAME}"
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

; AAE has to be closed before its files can be replaced. An update closes it
; itself; otherwise, the user is asked to.
!macro WaitForAAEToClose
    StrCpy $1 0
    loop:
        FindWindow $0 "AAEMain"
        IntCmp $0 0 done
        IfSilent 0 ask
            ; An update closes AAE as the installer starts; give it a moment.
            IntOp $1 $1 + 1
            IntCmp $1 30 gave_up 0 gave_up
            Sleep 500
            Goto loop
        ask:
            MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "${NAME} is running. Close it, then choose Retry." IDRETRY loop
            Abort
        gave_up:
            Abort
    done:
!macroend

Section "Install"
    !insertmacro WaitForAAEToClose
    SetOutPath "$INSTDIR"
    File /r "${STAGE}/*"
    WriteUninstaller "$INSTDIR\Uninstall.exe"

    CreateShortcut "$SMPROGRAMS\${NAME}.lnk" "$INSTDIR\${EXE}"

    WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayName" "${NAME}"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayVersion" "${VERSION}"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "Publisher" "aaron-gh"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "DisplayIcon" "$INSTDIR\${EXE}"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "InstallLocation" "$INSTDIR"
    WriteRegStr HKCU "${UNINSTALL_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
    WriteRegStr HKCU "${UNINSTALL_KEY}" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /S'
    WriteRegStr HKCU "${UNINSTALL_KEY}" "URLInfoAbout" "https://github.com/aaron-gh/accessible-android-emulator"
    WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoModify" 1
    WriteRegDWORD HKCU "${UNINSTALL_KEY}" "NoRepair" 1

    ; After a quiet update, AAE starts again, as it was before.
    IfSilent 0 +2
        Exec '"$INSTDIR\${EXE}"'
SectionEnd

Section "Uninstall"
    !insertmacro WaitForAAEToClose
    Delete "$SMPROGRAMS\${NAME}.lnk"
    ; Only AAE's own files, by name, in case it was installed in a folder
    ; with others. Devices, Android versions and settings stay in AAE's data
    ; folder.
    Delete "$INSTDIR\AccessibleAndroidEmulator.exe"
    Delete "$INSTDIR\aae.exe"
    Delete "$INSTDIR\aae-helper.apk"
    Delete "$INSTDIR\aae-espeak.apk"
    Delete "$INSTDIR\nvdaControllerClient.dll"
    Delete "$INSTDIR\WinSparkle.dll"
    Delete "$INSTDIR\README.txt"
    Delete "$INSTDIR\LICENSE.txt"
    Delete "$INSTDIR\nvdaControllerClient-LICENSE.txt"
    Delete "$INSTDIR\WinSparkle-LICENSE.txt"
    Delete "$INSTDIR\Uninstall.exe"
    RMDir "$INSTDIR"
    DeleteRegKey HKCU "${UNINSTALL_KEY}"
SectionEnd
