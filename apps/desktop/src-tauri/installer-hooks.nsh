; SPDX-License-Identifier: MPL-2.0
; Keep the legacy installer directory and MewuAI.exe shortcut/startup target.
; Do not uninstall the WPF application first: its local data remains available
; for the one-time, host-owned connection import.
Var MewuLegacyUpgrade
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
  ${If} $MewuLegacyUpgrade == 1
    ; Prevent the obsolete Inno uninstaller from deleting the replacement EXE.
    ; This removes only its registration, never application/user data files.
    DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\{D9760D1F-112A-4DC7-97F4-8F2D905C1A36}_is1"
  ${EndIf}
!macroend
