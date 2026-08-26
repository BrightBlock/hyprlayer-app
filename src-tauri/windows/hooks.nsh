
!macro NSIS_HOOK_POSTINSTALL
  ; Keep winget from matching the desktop app to the CLI package (#8894).
  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayName" "Hyprlayer Desktop"
!macroend
