; Only the visible, explicitly selected data cleanup path invokes the native helper.
; Never use RMDir /r here: app-owned trees can contain junctions into user projects.
!macro NSIS_HOOK_PREUNINSTALL
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    !insertmacro CheckIfAppIsRunning "$INSTDIR\${MAINBINARYNAME}.exe" "${PRODUCTNAME}"
    ClearErrors
    ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --purge-local-data' $0
    ${If} ${Errors}
      MessageBox MB_OK|MB_ICONSTOP "Unable to start local app data cleanup. Uninstallation stopped."
      Abort
    ${EndIf}
    ${If} $0 <> 0
      MessageBox MB_OK|MB_ICONSTOP "Local app data cleanup failed or the app is running. Uninstallation stopped."
      Abort
    ${EndIf}
    ; Skip Tauri's two unsafe recursive BUNDLEID deletions after the safe helper succeeded.
    StrCpy $DeleteAppDataCheckboxState 0
    DeleteRegKey SHCTX "${MANUPRODUCTKEY}"
    DeleteRegKey /ifempty SHCTX "${MANUKEY}"
    DeleteRegValue HKCU "${MANUPRODUCTKEY}" "Installer Language"
    DeleteRegKey /ifempty HKCU "${MANUPRODUCTKEY}"
    DeleteRegKey /ifempty HKCU "${MANUKEY}"
  ${EndIf}
!macroend
