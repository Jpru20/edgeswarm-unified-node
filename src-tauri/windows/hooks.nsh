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
!macroend