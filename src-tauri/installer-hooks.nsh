; NSIS hooks for the Windows installer (bundle.windows.nsis.installerHooks in tauri.conf.json).

!macro NSIS_HOOK_POSTINSTALL
  ; 1.5.0 shipped the command-line scan (now examples/scan.rs) before it was ready. Updates remove it.
  Delete "$INSTDIR\chillsweep-scan.exe"
!macroend
