; DeepSeek Local - Windows installer (NSIS 3).
;
; Build from windows/:
;   makensis /DAPP_VERSION=0.1.0 /DOUT_FILE=..\dist\DSLocal-0.1.0-windows-x64-setup.exe installer.nsi
Unicode true

!define APP_NAME "DeepSeek Local"
!define APP_EXE "dslocal.exe"
!define APP_PUBLISHER "tuean"
!define APP_URL "https://github.com/tuean/dslocal"
!define APP_UNINST_KEY "Software\Microsoft\Windows\CurrentVersion\Uninstall\DSLocal"

!ifndef APP_VERSION
  !define APP_VERSION "0.1.0"
!endif
!ifndef SRC_EXE
  !define SRC_EXE "target\release\dslocal.exe"
!endif
!ifndef OUT_FILE
  !define OUT_FILE "DSLocal-${APP_VERSION}-windows-x64-setup.exe"
!endif

VIProductVersion "${APP_VERSION}.0"
VIAddVersionKey /LANG=1033 "ProductName" "${APP_NAME}"
VIAddVersionKey /LANG=1033 "FileDescription" "${APP_NAME} installer"
VIAddVersionKey /LANG=1033 "FileVersion" "${APP_VERSION}"
VIAddVersionKey /LANG=1033 "ProductVersion" "${APP_VERSION}"
VIAddVersionKey /LANG=1033 "CompanyName" "${APP_PUBLISHER}"
VIAddVersionKey /LANG=1033 "LegalCopyright" "MIT"

Name "${APP_NAME}"
OutFile "${OUT_FILE}"
InstallDir "$LOCALAPPDATA\Programs\DeepSeek Local"
InstallDirRegKey HKCU "Software\DSLocal" "InstallDir"
RequestExecutionLevel user
SetCompressor /SOLID lzma

Icon "assets\icon.ico"
UninstallIcon "assets\icon.ico"

!include "MUI2.nsh"
!define MUI_ICON "assets\icon.ico"
!define MUI_UNICON "assets\icon.ico"
!define MUI_ABORTWARNING
!define MUI_FINISHPAGE_RUN "$INSTDIR\${APP_EXE}"
!define MUI_FINISHPAGE_RUN_TEXT "启动 ${APP_NAME}"

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "SimpChinese"
!insertmacro MUI_LANGUAGE "English"

Section "DeepSeek Local" SecMain
  SetOutPath "$INSTDIR"
  File "${SRC_EXE}"

  CreateShortcut "$SMPROGRAMS\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}" "" "$INSTDIR\${APP_EXE}" 0
  CreateShortcut "$DESKTOP\${APP_NAME}.lnk" "$INSTDIR\${APP_EXE}" "" "$INSTDIR\${APP_EXE}" 0

  WriteUninstaller "$INSTDIR\uninstall.exe"
  WriteRegStr HKCU "Software\DSLocal" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "${APP_UNINST_KEY}" "DisplayName" "${APP_NAME}"
  WriteRegStr HKCU "${APP_UNINST_KEY}" "DisplayIcon" "$INSTDIR\${APP_EXE}"
  WriteRegStr HKCU "${APP_UNINST_KEY}" "DisplayVersion" "${APP_VERSION}"
  WriteRegStr HKCU "${APP_UNINST_KEY}" "Publisher" "${APP_PUBLISHER}"
  WriteRegStr HKCU "${APP_UNINST_KEY}" "URLInfoAbout" "${APP_URL}"
  WriteRegStr HKCU "${APP_UNINST_KEY}" "UninstallString" '"$INSTDIR\uninstall.exe"'
  WriteRegDWORD HKCU "${APP_UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKCU "${APP_UNINST_KEY}" "NoRepair" 1
SectionEnd

Section "Uninstall"
  Delete "$INSTDIR\${APP_EXE}"
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
  Delete "$SMPROGRAMS\${APP_NAME}.lnk"
  Delete "$DESKTOP\${APP_NAME}.lnk"
  DeleteRegKey HKCU "${APP_UNINST_KEY}"
  DeleteRegKey HKCU "Software\DSLocal"
SectionEnd
