; Atalho para a pasta da extensão, que fica oculta em AppData.
!macro NSIS_HOOK_POSTINSTALL
  CreateShortCut "$SMPROGRAMS\${PRODUCTNAME} - pasta da extensão.lnk" "$INSTDIR\extension"
!macroend

; O app grava estes registros sozinho; o desinstalador padrão não os conhece.
!macro NSIS_HOOK_PREUNINSTALL
  nsExec::Exec '"$INSTDIR\${MAINBINARYNAME}.exe" unregister-host'
  Pop $0
  DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "dlm"
  Delete "$SMPROGRAMS\${PRODUCTNAME} - pasta da extensão.lnk"
!macroend
