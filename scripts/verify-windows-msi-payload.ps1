param([Parameter(Mandatory=$true)][string]$Msi,[Parameter(Mandatory=$true)][string]$TargetDir)
$ErrorActionPreference = 'Stop'
if (!(Test-Path -LiteralPath $Msi)) { throw 'msi_artifact_missing' }
$Audit = Join-Path $TargetDir 'msi-payload-audit'
Remove-Item $Audit -Recurse -Force -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Path $Audit | Out-Null
$P = Start-Process msiexec.exe -ArgumentList "/a `"$Msi`" TARGETDIR=`"$Audit`" /qn" -Wait -PassThru
if ($P.ExitCode -ne 0) { throw "msi_extract_failed_$($P.ExitCode)" }
$Payload = Get-ChildItem $Audit -Recurse -Filter 'edgeswarm-unified-node.exe' -File | Select-Object -First 1
if (!$Payload) { throw 'msi_payload_exe_missing' }
$Url = [string]$env:EDGESWARM_DEFAULT_SUPABASE_URL
$Key = [string]$env:EDGESWARM_DEFAULT_SUPABASE_ANON_KEY
if ([string]::IsNullOrWhiteSpace($Url) -or [string]::IsNullOrWhiteSpace($Key)) { throw 'compiled_config_env_missing_for_payload_check' }
$BinaryText = [Text.Encoding]::UTF8.GetString([IO.File]::ReadAllBytes($Payload.FullName))
if (!$BinaryText.Contains($Url)) { throw 'msi_payload_supabase_url_not_found' }
if (!$BinaryText.Contains($Key)) { throw 'msi_payload_supabase_anon_key_not_found' }
$Hash = (Get-FileHash $Payload.FullName -Algorithm SHA256).Hash
Write-Host 'MSI_PAYLOAD_CONFIG_VERIFIED=PASS'
Write-Host "CANONICAL_RUNTIME_PATH=$($Payload.FullName)"
Write-Host "CANONICAL_RUNTIME_SHA256=$Hash"
