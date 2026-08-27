; Минимальный per-user установщик для прогона Phase 0.1,
; если Tauri не смог скачать свой nsis-3.11.zip с GitHub.
Unicode true
Name "Serein"
OutFile "phase0-Serein_1.0.0_x64-setup.exe"
InstallDir "$LOCALAPPDATA\Serein"
RequestExecutionLevel user
SetCompressor lzma

!define REG "Software\Microsoft\Windows\CurrentVersion\Uninstall\Serein"

Page directory
Page instfiles
UninstPage uninstConfirm
UninstPage instfiles

Section "Install"
  SetOutPath "$INSTDIR"
  File "/oname=Serein.exe" "..\src-tauri\target\release\serein.exe"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  CreateShortCut "$SMPROGRAMS\Serein.lnk" "$INSTDIR\Serein.exe"
  WriteRegStr HKCU "${REG}" "DisplayName" "Serein"
  WriteRegStr HKCU "${REG}" "DisplayVersion" "1.0.0"
  WriteRegStr HKCU "${REG}" "Publisher" "HaDe"
  WriteRegStr HKCU "${REG}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "${REG}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegStr HKCU "${REG}" "QuietUninstallString" '"$INSTDIR\uninstall.exe" /S'
  WriteRegDWORD HKCU "${REG}" "NoModify" 1
  WriteRegDWORD HKCU "${REG}" "NoRepair" 1
SectionEnd

Section "Uninstall"
  Delete "$INSTDIR\Serein.exe"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\Serein.lnk"
  DeleteRegKey HKCU "${REG}"
SectionEnd
