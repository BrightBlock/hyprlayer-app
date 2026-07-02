; Included by Tauri's NSIS template via bundle > windows > nsis > installerHooks.
; Keep this file pure ASCII (see release-win-remote.ps1 header).

!macro NSIS_HOOK_POSTINSTALL
  ; Qualify the Add/Remove Programs title so winget stops correlating the
  ; desktop app with the CLI package BrightBlock.Hyprlayer (#8894). Runs after
  ; the template's own DisplayName write, so this value wins.
  WriteRegStr SHCTX "${UNINSTKEY}" "DisplayName" "Hyprlayer Desktop"
!macroend
