Unicode true
!include "MUI2.nsh"
!include "FileFunc.nsh"
!include "StrFunc.nsh"
!include "x64.nsh"
${StrStr}

!define PRODUCT "SmartTaskbar"
!define VERSION "0.1.0"
!define APP "smarttaskbar-rs.exe"
!define UNINSTALL "Software\Microsoft\Windows\CurrentVersion\Uninstall\SmartTaskbar"
!define RUN "Software\Microsoft\Windows\CurrentVersion\Run"
!define LEGACY_START "SmartTaskbar Rust"

Name "${PRODUCT}"
OutFile "${OUTPUT}"
Icon "${ICON}"
UninstallIcon "${ICON}"
InstallDir "$LOCALAPPDATA\Programs\SmartTaskbar"
RequestExecutionLevel user
SetCompressor /SOLID lzma
AutoCloseWindow true
ShowInstDetails show
VIProductVersion "0.1.0.0"
VIAddVersionKey "ProductName" "SmartTaskbar"
VIAddVersionKey "FileDescription" "SmartTaskbar Windows 11 Setup"
VIAddVersionKey "FileVersion" "${VERSION}"
VIAddVersionKey "LegalCopyright" "SmartTaskbar contributors. See included license notices."

Var Existing
Var Registered
Var Stage
Var ReplaceStyle
Var Reason

!define MUI_ABORTWARNING
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_COMPONENTS
!define MUI_PAGE_CUSTOMFUNCTION_PRE DirectoryPage
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\${APP}"
!define MUI_FINISHPAGE_RUN_PARAMETERS "--settings"
!define MUI_FINISHPAGE_RUN_TEXT "Open SmartTaskbar Settings"
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Function .onInit
    SetShellVarContext current
    SetRegView 64
    ${IfNot} ${IsNativeAMD64}
        MessageBox MB_ICONSTOP "SmartTaskbar requires Windows 11 on a 64-bit Intel or AMD PC." /SD IDOK
        Abort
    ${EndIf}
    ReadRegStr $0 HKLM "Software\Microsoft\Windows NT\CurrentVersion" "CurrentBuildNumber"
    IntCmp $0 22000 detect unsupported detect
    unsupported:
        MessageBox MB_ICONSTOP "SmartTaskbar requires Windows 11." /SD IDOK
        Abort
    detect:
        ReadRegStr $Registered HKCU "${UNINSTALL}" "InstallLocation"
        StrCmp $Registered "" legacy
        IfFileExists "$Registered\${APP}" 0 legacy
        StrCpy $Existing $Registered
        Goto use_existing
    legacy:
        # Older portable builds registered the quoted executable when sign-in was enabled.
        ReadRegStr $0 HKCU "${RUN}" "${LEGACY_START}"
        StrCpy $1 $0 1
        StrCmp $1 '"' 0 done
        StrCpy $1 $0 "" 1
        ${StrStr} $2 $1 '"'
        StrCmp $2 "" done
        StrLen $0 $1
        StrLen $2 $2
        IntOp $0 $0 - $2
        StrCpy $1 $1 $0
        ${GetFileName} $1 $2
        StrCmp $2 "${APP}" 0 done
        IfFileExists "$1" 0 done
        ${GetParent} $1 $Existing
    use_existing:
        StrCpy $INSTDIR $Existing
    done:
FunctionEnd

Function DirectoryPage
    StrCmp $Existing "" 0 skip_page
    Return
    skip_page:
        Abort
FunctionEnd

Function .onVerifyInstDir
    IfFileExists "$INSTDIR\Windhawk\windhawk.exe" 0 okay
    StrCmp $INSTDIR $Registered okay
    Abort
    okay:
FunctionEnd

Section "SmartTaskbar (required)" Application
    SectionIn RO
    System::Call 'kernel32::CreateMutexW(p 0, i 0, w "Local\SmartTaskbar.Setup") p .r0 ?e'
    Pop $1
    StrCmp $0 0 busy_setup
    StrCmp $1 183 busy_setup
    IfFileExists "$INSTDIR\Windhawk\windhawk.exe" 0 prepare
    StrCmp $INSTDIR $Registered prepare
    MessageBox MB_ICONSTOP "Choose a folder that has no existing Windhawk installation." /SD IDOK
    SetErrorLevel 2
    Abort
    busy_setup:
        MessageBox MB_ICONSTOP "SmartTaskbar Setup is already running." /SD IDOK
        SetErrorLevel 2
        Abort
    prepare:
        StrCpy $ReplaceStyle 1
        StrCpy $Reason "Unpacking the new app and styling files"
        SetOutPath "$INSTDIR"
        ClearErrors
        GetTempFileName $Stage "$INSTDIR"
        IfErrors failed
        Delete "$Stage"
        SetOutPath "$Stage"
        File /r "${PAYLOAD}\App\*"
        File /r "${PAYLOAD}\Windhawk"
        IfErrors failed
        StrCpy $Reason "Validating the bundled styling files"
        ExecWait '"$Stage\${APP}" --configure-style' $0
        IfErrors failed
        StrCmp $0 0 0 failed
        # Validate the complete new payload before interrupting the current desktop.
        ExecWait '"$Stage\${APP}" --prepare-update --config "$INSTDIR\settings.json"' $0
        IfErrors running
        StrCmp $0 0 0 running
        IfFileExists "$INSTDIR\shell-restore.json" running
        SetOutPath "$INSTDIR"
        CreateDirectory "$Stage\prior"
        IfFileExists "$INSTDIR\${APP}" 0 existing_style
        StrCpy $Reason "Moving the previous app"
        ClearErrors
        Rename "$INSTDIR\${APP}" "$Stage\prior\${APP}"
        IfErrors restore
    existing_style:
        IfFileExists "$INSTDIR\Windhawk\windhawk.exe" 0 swap_app
        # Keep the same verified portable style version and its caches on app updates.
        ExecWait '"$Stage\${APP}" --verify-style --config "$INSTDIR\settings.json"' $0
        IfErrors backup_style
        StrCmp $0 0 reuse_style backup_style
    reuse_style:
        StrCpy $ReplaceStyle 0
        Goto swap_app
    backup_style:
        StrCpy $Reason "Moving the previous styling host"
        ClearErrors
        Rename "$INSTDIR\Windhawk" "$Stage\prior\Windhawk"
        IfErrors restore
    swap_app:
        StrCpy $Reason "Putting the new app in place"
        ClearErrors
        Rename "$Stage\${APP}" "$INSTDIR\${APP}"
        IfErrors restore
        StrCmp $ReplaceStyle 0 copy_docs
        StrCpy $Reason "Putting the new styling host in place"
        Rename "$Stage\Windhawk" "$INSTDIR\Windhawk"
        IfErrors restore_app
    copy_docs:
        StrCpy $Reason "Copying license and documentation files"
        CopyFiles /SILENT "$Stage\licenses" "$INSTDIR"
        CopyFiles /SILENT "$Stage\LICENSE-SmartTaskbar.txt" "$INSTDIR"
        CopyFiles /SILENT "$Stage\NOTICES.md" "$INSTDIR"
        CopyFiles /SILENT "$Stage\README.md" "$INSTDIR"
        IfErrors restore_app_and_style
        StrCpy $Reason "Restoring the sign-in preference"
        ExecWait '"$INSTDIR\${APP}" --sync-startup' $0
        IfErrors restore_app_and_style
        StrCmp $0 0 register restore_app_and_style
    restore_app_and_style:
        StrCmp $ReplaceStyle 0 restore_app
        Rename "$INSTDIR\Windhawk" "$Stage\Windhawk"
    restore_app:
        Delete "$INSTDIR\${APP}"
    restore:
        IfFileExists "$Stage\prior\Windhawk\windhawk.exe" 0 restore_exe
        Rename "$Stage\prior\Windhawk" "$INSTDIR\Windhawk"
    restore_exe:
        IfFileExists "$Stage\prior\${APP}" 0 resume
        Rename "$Stage\prior\${APP}" "$INSTDIR\${APP}"
    resume:
        IfFileExists "$INSTDIR\${APP}" 0 failed
        Exec '"$INSTDIR\${APP}" --run'
        Goto failed
    running:
        SetOutPath "$INSTDIR"
        RMDir /r "$Stage"
        MessageBox MB_ICONSTOP "SmartTaskbar could not finish restoring the Windows taskbar. Close any open SmartTaskbar settings and try again." /SD IDOK
        SetErrorLevel 2
        Abort
    failed:
        SetOutPath "$INSTDIR"
        StrCmp $Stage "" show_failure
        IfFileExists "$Stage\prior\${APP}" show_failure
        IfFileExists "$Stage\prior\Windhawk\windhawk.exe" show_failure
        RMDir /r "$Stage"
    show_failure:
        FileOpen $0 "$INSTDIR\setup-error.txt" w
        IfErrors report_failure
        FileWrite $0 "$Reason failed.$\r$\n"
        FileClose $0
    report_failure:
        MessageBox MB_ICONSTOP "$Reason failed." /SD IDOK
        SetErrorLevel 3
        Abort
    register:
        WriteUninstaller "$INSTDIR\Uninstall.exe"
        Delete "$INSTDIR\setup-error.txt"
        RMDir /r "$Stage"
        WriteRegStr HKCU "${UNINSTALL}" "DisplayName" "${PRODUCT}"
        WriteRegStr HKCU "${UNINSTALL}" "DisplayVersion" "${VERSION}"
        WriteRegStr HKCU "${UNINSTALL}" "DisplayIcon" '"$INSTDIR\Uninstall.exe",0'
        WriteRegStr HKCU "${UNINSTALL}" "InstallLocation" "$INSTDIR"
        WriteRegStr HKCU "${UNINSTALL}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
        WriteRegStr HKCU "${UNINSTALL}" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /S'
        WriteRegDWORD HKCU "${UNINSTALL}" "NoModify" 1
        WriteRegDWORD HKCU "${UNINSTALL}" "NoRepair" 1
        CreateShortcut "$SMPROGRAMS\SmartTaskbar.lnk" "$INSTDIR\${APP}" "--settings" "$INSTDIR\Uninstall.exe"
        Delete "$SMPROGRAMS\SmartTaskbar Rust.lnk"
        Delete "$DESKTOP\SmartTaskbar Rust.lnk"
        Exec '"$INSTDIR\${APP}" --run'
SectionEnd

Section "Desktop shortcut" DesktopShortcut
    CreateShortcut "$DESKTOP\SmartTaskbar.lnk" "$INSTDIR\${APP}" "--settings" "$INSTDIR\Uninstall.exe"
SectionEnd

Function un.onInit
    SetShellVarContext current
    SetRegView 64
    ReadRegStr $0 HKCU "${UNINSTALL}" "InstallLocation"
    StrCmp $0 $INSTDIR owned
    Abort
    owned:
FunctionEnd

Section "Uninstall"
    ExecWait '"$INSTDIR\${APP}" --prepare-update' $0
    StrCmp $0 0 0 busy
    IfFileExists "$INSTDIR\shell-restore.json" busy remove_app
    busy:
        MessageBox MB_ICONSTOP "Close SmartTaskbar and try again after it restores the Windows taskbar." /SD IDOK
        SetErrorLevel 2
        Abort
    remove_app:
        ReadRegStr $0 HKCU "${RUN}" "${LEGACY_START}"
        StrCmp $0 '"$INSTDIR\${APP}" --run --config "$INSTDIR\settings.json"' 0 +2
        DeleteRegValue HKCU "${RUN}" "${LEGACY_START}"
        DeleteRegKey HKCU "${UNINSTALL}"
        Delete "$SMPROGRAMS\SmartTaskbar.lnk"
        Delete "$DESKTOP\SmartTaskbar.lnk"
        Delete "$INSTDIR\${APP}"
        Delete "$INSTDIR\LICENSE-SmartTaskbar.txt"
        Delete "$INSTDIR\NOTICES.md"
        Delete "$INSTDIR\README.md"
        RMDir /r "$INSTDIR\licenses"
        RMDir /r "$INSTDIR\Windhawk"
        Delete "$INSTDIR\Uninstall.exe"
        RMDir "$INSTDIR"
SectionEnd
