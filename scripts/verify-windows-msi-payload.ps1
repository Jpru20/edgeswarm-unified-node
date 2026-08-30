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
$ExpectedLlama = "973ff5fd98d0ffe335761c304ac9faa365e57877054f764ac0c40a98a70cf717"
$Llama = Get-ChildItem $Audit -Recurse -Filter "llama-server.exe" -File |
    Select-Object -First 1

if (!$Llama) { throw "msi_payload_llama_server_missing" }

$LlamaHash = (Get-FileHash $Llama.FullName -Algorithm SHA256).Hash.ToLower()
if ($LlamaHash -ne $ExpectedLlama) {
    throw "msi_payload_llama_sha_mismatch"
}

if ($Llama.FullName -notmatch "runtime\\current\\llama-server\.exe$") {
    throw "msi_payload_llama_path_invalid"
}

Write-Host "MSI_LLAMA_RUNTIME_PATH=$($Llama.FullName)"
Write-Host "MSI_LLAMA_RUNTIME_SHA256=$LlamaHash"
Write-Host "MSI_BUNDLED_LLAMA_RUNTIME=PASS"
