; SPDX-License-Identifier: MPL-2.0
; Keep the legacy installer directory and MewuAI.exe shortcut/startup target.
; Do not uninstall the WPF application first: its local data remains available
; for the one-time, host-owned connection import.
Var MewuLegacyUpgrade
!define MEWU_CLI_PATH_SCRIPT "${__FILEDIR__}\resources\cli\integrate-windows.ps1"
LangString MewuCliPathFailed ${LANG_ENGLISH} "Mewu is installed, but CLI PATH setup failed ($0). Re-run this installer to repair it. CLI: $INSTDIR\mewu.exe"
LangString MewuCliPathFailed ${LANG_SIMPCHINESE} "Mewu 已安装，但命令行 PATH 配置失败（$0）。可重新运行安装器修复。命令行程序：$INSTDIR\mewu.exe"

Function MewuInstallCliPath
  Push $0
  Push $1
  InitPluginsDir
  SetOutPath "$PLUGINSDIR"
  File /oname=mewu-cli-path.ps1 "${MEWU_CLI_PATH_SCRIPT}"
  nsExec::ExecToStack /TIMEOUT=60000 '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\mewu-cli-path.ps1" -Mode Install -InstallDirectory "$INSTDIR"'
  Pop $0
  Pop $1
  ${If} $0 != "0"
    DetailPrint "Mewu CLI PATH registration failed ($0): $1"
    MessageBox MB_OK|MB_ICONEXCLAMATION "$(MewuCliPathFailed)" /SD IDOK
  ${EndIf}
  SetOutPath "$INSTDIR"
  Pop $1
  Pop $0
FunctionEnd

Function un.MewuUninstallCliPath
  Push $0
  Push $1
  InitPluginsDir
  SetOutPath "$PLUGINSDIR"
  File /oname=mewu-cli-path.ps1 "${MEWU_CLI_PATH_SCRIPT}"
  nsExec::ExecToStack /TIMEOUT=60000 '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoLogo -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "$PLUGINSDIR\mewu-cli-path.ps1" -Mode Uninstall -InstallDirectory "$INSTDIR"'
  Pop $0
  Pop $1
  ${If} $0 != "0"
    DetailPrint "Mewu CLI could not remove its owned user PATH entry."
  ${EndIf}
  Pop $1
  Pop $0
FunctionEnd
!macro NSIS_HOOK_PREINSTALL
  ReadRegStr $R1 HKCU "${UNINSTKEY}" "InstallLocation"
  ${If} $R1 == ""
    ReadRegStr $R0 HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\{D9760D1F-112A-4DC7-97F4-8F2D905C1A36}_is1" "InstallLocation"
    ${If} $R0 != ""
      ${If} ${FileExists} "$R0\MewuAI.exe"
        StrCpy $INSTDIR $R0
        SetOutPath $INSTDIR
        StrCpy $MewuLegacyUpgrade 1
      ${EndIf}
    ${EndIf}
  ${EndIf}
!macroend
!macro NSIS_HOOK_POSTINSTALL
  ; Cargo's ordinary mewu binary is bundled alongside MewuAI by Tauri. Do not
  ; ship a second externalBin copy or accidentally package the dev validator.
  ${IfNot} ${FileExists} "$INSTDIR\mewu.exe"
    Abort "The Mewu CLI executable is missing from this installation."
  ${EndIf}
  Call MewuInstallCliPath
  ${If} $MewuLegacyUpgrade == 1
    ; Prevent the obsolete Inno uninstaller from deleting the replacement EXE.
    ; This removes only its registration, never application/user data files.
    DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\{D9760D1F-112A-4DC7-97F4-8F2D905C1A36}_is1"
  ${EndIf}
!macroend
!macro NSIS_HOOK_PREUNINSTALL
  Call un.MewuUninstallCliPath
!macroend
