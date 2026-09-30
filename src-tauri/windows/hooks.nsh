!macro NSIS_HOOK_PREINSTALL
  ; WINDOWS_PROVIDER_PREINSTALL_STOP_V1
  ;
  ; Stop the old background provider before NSIS replaces the
  ; supervisor/headless binaries. The updater task itself is deliberately
  ; left intact because it may be the process driving this installation.
  DetailPrint "Stopping existing EdgeSwarm provider before file replacement..."

  ; WINDOWS_UPDATE_PAUSE_LOCK_PREINSTALL_V1
  ;
  ; Prevent the one-minute supervisor watchdog from recreating
  ; provider processes while installer files are being replaced.
  ClearErrors
  FileOpen $5 "$INSTDIR\update-pause.lock" w

  ${If} ${Errors}
    MessageBox MB_ICONSTOP \
      "EdgeSwarm could not create the update pause lock."
    Abort
  ${EndIf}

  FileWrite $5 "nsis-preinstall"
  FileClose $5

  DetailPrint "EdgeSwarm update pause lock active."

  ExecWait \
    '"$SYSDIR\schtasks.exe" /End /TN "EdgeSwarm Node Supervisor"' \
    $4

  ; WINDOWS_PROVIDER_TASK_PRESERVE_PREINSTALL_V1
  ;
  ; Preserve the task registration during an in-place application update.
  ; The existing action continues to point at the same installed supervisor
  ; path and the post-install step can refresh/restart it. Deletion is
  ; reserved for the real uninstall path.

  ExecWait \
    '"$SYSDIR\taskkill.exe" /F /T /IM edgeswarm-node-supervisor.exe' \
    $4

  ExecWait \
    '"$SYSDIR\taskkill.exe" /F /T /IM edgeswarm-node-headless.exe' \
    $4

  Sleep 1000

  ; WINDOWS_MAIN_BINARY_RELEASE_GATE_V1
  ;
  ; Tauri launches NSIS and then exits the old application.
  ; Before copying the new main executable, wait until Windows
  ; has actually released the old image mapping. This closes
  ; the launch/exit race without relying on a blind long sleep.
  Push $R8
  Push $R9

  ${IfNot} ${FileExists} "$INSTDIR\edgeswarm-unified-node.exe"
    Goto edgeswarm_main_binary_released_v1
  ${EndIf}

  StrCpy $R8 0

edgeswarm_wait_main_binary_v1:
  ClearErrors
  FileOpen $R9 "$INSTDIR\edgeswarm-unified-node.exe" a

  ${IfNot} ${Errors}
    FileClose $R9
    Goto edgeswarm_main_binary_released_v1
  ${EndIf}

  IntOp $R8 $R8 + 1

  ${If} $R8 < 20
    Sleep 250
    Goto edgeswarm_wait_main_binary_v1
  ${EndIf}

  ; After a five-second graceful-release window, any remaining
  ; process with this exact image name is stale for installation.
  ; Do not use /T here: the installer itself must never be killed
  ; as a descendant of the old application process.
  DetailPrint "EdgeSwarm application binary still busy; closing stale application process..."

  ExecWait \
    '"$SYSDIR\taskkill.exe" /F /IM edgeswarm-unified-node.exe' \
    $4

  StrCpy $R8 0

edgeswarm_wait_main_binary_after_kill_v1:
  ClearErrors
  FileOpen $R9 "$INSTDIR\edgeswarm-unified-node.exe" a

  ${IfNot} ${Errors}
    FileClose $R9
    Goto edgeswarm_main_binary_released_v1
  ${EndIf}

  IntOp $R8 $R8 + 1

  ${If} $R8 < 20
    Sleep 250
    Goto edgeswarm_wait_main_binary_after_kill_v1
  ${EndIf}

  Pop $R9
  Pop $R8

  MessageBox MB_ICONSTOP \
    "EdgeSwarm could not release the existing application binary for update."

  Abort

edgeswarm_main_binary_released_v1:
  Pop $R9
  Pop $R8

  DetailPrint "EdgeSwarm application binary released for replacement."
  DetailPrint "Existing EdgeSwarm provider stopped."
!macroend

!macro NSIS_HOOK_POSTINSTALL
  SetRegView 64
  ClearErrors

  ReadRegDWord $0 HKLM \
    "SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64" \
    "Installed"

  ${If} $0 == 1
    DetailPrint "Microsoft Visual C++ Runtime already installed."
  ${Else}
    DetailPrint "Installing Microsoft Visual C++ Runtime..."

    ExecWait \
      '"$INSTDIR\resources\windows\vc_redist.x64.exe" /install /quiet /norestart' \
      $1

    ${If} $1 == 0
      DetailPrint "Microsoft Visual C++ Runtime installed."
    ${ElseIf} $1 == 1638
      DetailPrint "Microsoft Visual C++ Runtime already present."
    ${ElseIf} $1 == 3010
      DetailPrint "Microsoft Visual C++ Runtime installed; reboot requested."
    ${Else}
      MessageBox MB_ICONSTOP \
        "Microsoft Visual C++ Runtime installation failed. Exit code: $1"
      Abort
    ${EndIf}
  ${EndIf}

  ; WINDOWS_SUPERVISOR_TASK_POSTINSTALL_V1
  DetailPrint "Configuring EdgeSwarm background supervisor..."

  ${IfNot} ${FileExists} "$INSTDIR\edgeswarm-node-supervisor.exe"
    MessageBox MB_ICONSTOP \
      "EdgeSwarm supervisor binary is missing from the installation."
    Abort
  ${EndIf}

  ${IfNot} ${FileExists} "$INSTDIR\resources\windows\supervisor-task.ps1"
    MessageBox MB_ICONSTOP \
      "EdgeSwarm supervisor task configuration is missing from the installation."
    Abort
  ${EndIf}

  ; WINDOWS_SUPERVISOR_POSTINSTALL_DIAGNOSTIC_RETRY_V3
  ;
  ; Capture the exact PowerShell output produced inside the
  ; NSIS process context and retry the complete idempotent
  ; supervisor configuration when a transient invocation fails.
  Delete "$INSTDIR\supervisor-install.stdout.log"
  Delete "$INSTDIR\supervisor-install.stderr.log"

  StrCpy $3 0

edgeswarm_supervisor_install_retry_v3:
  IntOp $3 $3 + 1

  DetailPrint "EdgeSwarm supervisor configuration attempt $3 of 3..."

  ExecWait \
    '"$SYSDIR\cmd.exe" /D /S /C ""$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -File "$INSTDIR\resources\windows\supervisor-task.ps1" -Mode Install -SupervisorPath "$INSTDIR\edgeswarm-node-supervisor.exe" -AppPath "$INSTDIR\edgeswarm-unified-node.exe" >> "$INSTDIR\supervisor-install.stdout.log" 2>> "$INSTDIR\supervisor-install.stderr.log""' \
    $2

  ${If} $2 == 0
    Goto edgeswarm_supervisor_install_ready_v3
  ${EndIf}

  DetailPrint "EdgeSwarm supervisor configuration attempt $3 failed. Exit code: $2"

  ${If} $3 < 3
    Sleep 2000
    Goto edgeswarm_supervisor_install_retry_v3
  ${EndIf}

  MessageBox MB_ICONSTOP \
    "EdgeSwarm background supervisor configuration failed after 3 attempts. Exit code: $2$\r$\n$\r$\nDiagnostic logs:$\r$\n$INSTDIR\supervisor-install.stdout.log$\r$\n$INSTDIR\supervisor-install.stderr.log"
  Abort

edgeswarm_supervisor_install_ready_v3:
  DetailPrint "EdgeSwarm background supervisor configured."
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; WINDOWS_SUPERVISOR_TASK_PREUNINSTALL_V1
  DetailPrint "Stopping EdgeSwarm background supervisor..."

  ${If} ${FileExists} "$INSTDIR\resources\windows\supervisor-task.ps1"
    ExecWait \
      '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -File "$INSTDIR\resources\windows\supervisor-task.ps1" -Mode Remove -SupervisorPath "$INSTDIR\edgeswarm-node-supervisor.exe" -AppPath "$INSTDIR\edgeswarm-unified-node.exe"' \
      $3

    ${If} $3 != 0
      MessageBox MB_ICONSTOP \
        "EdgeSwarm background supervisor could not be removed. Exit code: $3"
      Abort
    ${EndIf}
  ${EndIf}

  DetailPrint "EdgeSwarm background supervisor stopped."

  ; WINDOWS_UNINSTALL_AUTH_CLEANUP_V1
  ;
  ; A real uninstall must not leave a restorable authenticated
  ; session or wallet restart credential behind. Keep models,
  ; identity, certificates and other provider data intact.
  DetailPrint "Clearing EdgeSwarm authentication state..."

  Delete "$LOCALAPPDATA\EdgeSwarm\unified-node\auth_session.json"
  Delete "$LOCALAPPDATA\EdgeSwarm\unified-node\.auth_session.*.tmp"
  Delete "$LOCALAPPDATA\EdgeSwarm\unified-node\wallet_restart_credential.dpapi"

  ; Remove persisted runtime intent/status as well. Missing desired
  ; state fails closed to user_stopped on a later reinstall.
  Delete "$LOCALAPPDATA\EdgeSwarm\unified-node\desired_state.json"
  Delete "$LOCALAPPDATA\EdgeSwarm\unified-node\node_status.json"

  DetailPrint "EdgeSwarm authentication state cleared."
!macroend
