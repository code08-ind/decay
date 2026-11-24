; Decay's additions to Tauri's NSIS installer (bundle > windows > nsis > installerHooks;
; Docs/Release.md "Windows").
;
; - decay-host.exe, the browsers' native messaging host, goes next to Decay.exe: the app
;   registers that path with each browser at every start (Software Design §4.8). It's
;   built and staged at src-tauri\sidecar\ by scripts/sidecar.mjs (beforeBundleCommand,
;   which also signs it), so it's added here rather than as a resource (a resource has to
;   exist before the app is compiled).
; - A running decay-host.exe (a browser holds one per connected extension) would keep
;   the file locked: it's ended first. The extension reconnects by itself.
; - Uninstall (not an update) removes the browsers' registry entries for the host, and,
;   with "Delete the application data" ticked, Decay's data folder (%APPDATA%\Decay).

; Where this file is: the staged host is found relative to it at build time.
!define DECAY_HOOKS_DIR "${__FILEDIR__}"

!macro DECAY_END_HOST
  nsExec::Exec '"$SYSDIR\taskkill.exe" /F /IM decay-host.exe'
  Pop $0
  ; Let Windows release the file.
  Sleep 300
!macroend

!macro NSIS_HOOK_PREINSTALL
  !insertmacro DECAY_END_HOST
!macroend

!macro NSIS_HOOK_POSTINSTALL
  SetOutPath "$INSTDIR"
  File "/oname=decay-host.exe" "${DECAY_HOOKS_DIR}\..\sidecar\decay-host.exe"
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro DECAY_END_HOST
  Delete "$INSTDIR\decay-host.exe"
  ${If} $UpdateMode <> 1
    ; The browsers Decay registers with (crates/decay-host/src/install.rs, TARGETS).
    DeleteRegKey HKCU "Software\Google\Chrome\NativeMessagingHosts\com.decay.host"
    DeleteRegKey HKCU "Software\Chromium\NativeMessagingHosts\com.decay.host"
    DeleteRegKey HKCU "Software\Microsoft\Edge\NativeMessagingHosts\com.decay.host"
    DeleteRegKey HKCU "Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\com.decay.host"
    DeleteRegKey HKCU "Software\Mozilla\NativeMessagingHosts\com.decay.host"
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    SetShellVarContext current
    ; The archive database, backups, logs and the host's manifest. The database key
    ; stays in Credential Manager (com.decay.app) and is useless without the data.
    RMDir /r "$APPDATA\Decay"
  ${EndIf}
!macroend
