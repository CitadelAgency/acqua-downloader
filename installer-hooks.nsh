; Shortcut to the extension folder, which is hidden under AppData.
!macro NSIS_HOOK_POSTINSTALL
  CreateShortCut "$SMPROGRAMS\${PRODUCTNAME} - Browser Extension.lnk" "$INSTDIR\extension"
!macroend

; The app writes these keys itself, so the default uninstaller misses them.
!macro NSIS_HOOK_PREUNINSTALL
  nsExec::Exec '"$INSTDIR\${MAINBINARYNAME}.exe" unregister-host'
  Pop $0
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "dlm"
  Delete "$SMPROGRAMS\${PRODUCTNAME} - Browser Extension.lnk"
!macroend
