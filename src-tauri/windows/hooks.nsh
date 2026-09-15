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

  ExecWait \
    '"$SYSDIR\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -File "$INSTDIR\resources\windows\supervisor-task.ps1" -Mode Install -SupervisorPath "$INSTDIR\edgeswarm-node-supervisor.exe" -AppPath "$INSTDIR\edgeswarm-unified-node.exe"' \
    $2

  ${If} $2 != 0
    MessageBox MB_ICONSTOP \
      "EdgeSwarm background supervisor configuration failed. Exit code: $2"
    Abort
  ${EndIf}

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
!macroend
