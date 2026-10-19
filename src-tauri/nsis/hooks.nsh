; Uninstall hooks for the NSIS installer.
;
; The rule: installing over an install keeps everything. Settings (%APPDATA%\meteor),
; data, history, logs, downloaded models and the staged relay (%LOCALAPPDATA%\meteor,
; which is also the install folder), the web view's data (%LOCALAPPDATA%\dev.meteor.app),
; the keys in the Windows Credential Manager, the startup entry and METEOR V1's hooks in
; the agents' own config files are only ever taken away by a real uninstall, and the
; user's own data only when they tick "Delete the application data" there.
;
; The uninstaller runs in three ways, and only the first is a real uninstall:
;  1. The user starts it (Settings, Apps; or uninstall.exe by hand). NSIS then runs a
;     copy of it out of %TEMP%: $EXEDIR is not the install folder.
;  2. The installer runs it with /UPDATE (an update): $UpdateMode is 1.
;  3. The installer runs it for "Uninstall before installing" on its page, which is
;     the choice already made when the version differs, and what its passive mode
;     (/P) does over the same version. It is started in place
;     ("$INSTDIR\uninstall.exe" _?=$INSTDIR) and without /UPDATE, so $UpdateMode
;     cannot tell it from 1: $EXEDIR being $INSTDIR does. (Checked with a probe built
;     by the same NSIS.)
;
; A real uninstall, box ticked or not, takes out what would be left pointing at a
; program that is gone: the app's own entries in the agent CLIs' config files
; (`meteor.exe --uninstall-hooks`, lib.rs: an agent that fails closed on a hook command
; that no longer exists would otherwise refuse every tool call; only METEOR V1's entries
; go, each file gets the same dated backup as a click in Settings), the staged relay,
; the shelf's "Add to Shelf" entries, and the identity package (sparse/AppxManifest.xml)
; with the copy of its manifest, when the user registered it from Settings,
; Notifications and accessories. The template itself removes the startup entry.
;
; With the box ticked it also takes the user's own: the keys the app saved in the
; Windows Credential Manager (target "<key>.meteor", the way the keyring crate names
; them; the list is KNOWN_KEYS of crates/core/src/secrets.rs; keys a feature saved as
; "<id>/<name>" cannot be listed from here and stay), the settings folder and the
; data folder. The template removes the web view's data on the same tick.
;
; On 2 and 3 nothing is removed here, a tick of that box is not followed (it would
; cost the web view's data), and the startup entry the template takes out on 3 is put
; back as it was.
; (Ported from upstream `windows/src-tauri/nsis/hooks.nsh`, pull request 445.)

Var NotchReal ; 1: a real uninstall. 0: an update or a reinstall.
Var NotchRun  ; the startup entry as it was

!macro METEOR_CMDKEY name
  nsExec::Exec 'cmdkey /delete:${name}.meteor'
  Pop $0
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  StrCpy $NotchReal 1
  ${If} $UpdateMode = 1
  ${OrIf} $EXEDIR == $INSTDIR
    StrCpy $NotchReal 0
    StrCpy $DeleteAppDataCheckboxState 0
  ${EndIf}
  ReadRegStr $NotchRun HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "meteor"

  ${If} $NotchReal = 1
    ${If} ${FileExists} "$INSTDIR\meteor.exe"
      ExecWait '"$INSTDIR\meteor.exe" --uninstall-hooks'
    ${EndIf}
    ; The shelf's opt-in "Add to Shelf" entries: the Send To shortcut carries the
    ; label the key's default value holds. Untested: no uninstall was run.
    ReadRegStr $0 HKCU "Software\Classes\*\shell\meteor.AddToShelf" ""
    ${If} $0 != ""
      Delete "$SENDTO\$0.lnk"
    ${EndIf}
    DeleteRegKey HKCU "Software\Classes\*\shell\meteor.AddToShelf"
    DeleteRegKey HKCU "Software\Classes\Directory\shell\meteor.AddToShelf"
    ; The identity package of notification mirroring. Nothing happens when it was
    ; never registered. Untested: no uninstall was run.
    nsExec::Exec `powershell.exe -NoProfile -NonInteractive -Command "Get-AppxPackage -Name dev.meteor.app | Remove-AppxPackage"`
    Pop $0
    Delete "$LOCALAPPDATA\meteor\features\notifications-accessories\AppxManifest.xml"
    Delete "$LOCALAPPDATA\meteor\features\notifications-accessories\logo.png"
    ; The relay the app staged at launch: the installer never recorded it.
    RMDir /r "$LOCALAPPDATA\meteor\bin"
  ${EndIf}
!macroend

; After the template has removed the program's own files.
!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $NotchReal = 1
    ${If} $DeleteAppDataCheckboxState = 1
      !insertmacro METEOR_CMDKEY anthropic-api-key
      !insertmacro METEOR_CMDKEY openai-api-key
      !insertmacro METEOR_CMDKEY google-api-key
      !insertmacro METEOR_CMDKEY openrouter-api-key
      !insertmacro METEOR_CMDKEY openai-compatible-key
      !insertmacro METEOR_CMDKEY n8n-url
      !insertmacro METEOR_CMDKEY n8n-api-key
      !insertmacro METEOR_CMDKEY vercel-token
      !insertmacro METEOR_CMDKEY github-token
      !insertmacro METEOR_CMDKEY stripe-api-key
      !insertmacro METEOR_CMDKEY resend-api-key
      !insertmacro METEOR_CMDKEY notion-api-key
      !insertmacro METEOR_CMDKEY calcom-api-key
      ${If} $APPDATA != ""
        RMDir /r "$APPDATA\meteor"
      ${EndIf}
      ${If} $LOCALAPPDATA != ""
        RMDir /r "$LOCALAPPDATA\meteor"
      ${EndIf}
    ${EndIf}
  ${ElseIf} $NotchRun != ""
    WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "meteor" $NotchRun
  ${EndIf}
!macroend
