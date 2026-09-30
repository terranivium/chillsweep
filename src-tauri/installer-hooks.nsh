; NSIS hooks for the Windows installer (bundle.windows.nsis.installerHooks in tauri.conf.json).

!macro NSIS_HOOK_POSTINSTALL
  ; 1.5.0 shipped a developer command-line tool by mistake. Updates remove it.
  Delete "$INSTDIR\chillsweep-scan.exe"
!macroend
