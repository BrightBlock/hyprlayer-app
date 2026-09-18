!macro NSIS_HOOK_PREINSTALL
  ; The server outlives the app, so its exe is usually running here, and
  ; Windows lets a running exe be renamed but not overwritten. Park it; the new
  ; app flags the parked server as outdated until the user restarts it.
  Delete "$INSTDIR\hyprlayer-server.exe.old"
  Rename "$INSTDIR\hyprlayer-server.exe" "$INSTDIR\hyprlayer-server.exe.old"
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; Keep winget from matching the desktop app to the CLI package (#8894).
  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayName" "Hyprlayer Desktop"
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; The template only stops the main exe; a live server keeps its own exe, and
  ; so the install dir, from being deleted.
  !if "${INSTALLMODE}" == "currentUser"
    nsis_tauri_utils::KillProcessCurrentUser "hyprlayer-server.exe"
  !else
    nsis_tauri_utils::KillProcess "hyprlayer-server.exe"
  !endif
  Pop $R0
  Sleep 500
  Delete "$INSTDIR\hyprlayer-server.exe.old"
!macroend
