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

# WINDOWS_ACCELERATED_RUNTIME_PAYLOAD_VERIFY_V2
$CudaServer =
    Get-ChildItem $Audit -Recurse -Filter "llama-server.exe" -File |
    Where-Object {
        $_.FullName -match "resources\\runtime\\cuda\\current\\llama-server\.exe$"
    } |
    Select-Object -First 1

$CudaBackend =
    Get-ChildItem $Audit -Recurse -Filter "ggml-cuda.dll" -File |
    Select-Object -First 1

$CudaBlas =
    Get-ChildItem $Audit -Recurse -Filter "cublas64_12.dll" -File |
    Select-Object -First 1

$CudaBlasLt =
    Get-ChildItem $Audit -Recurse -Filter "cublasLt64_12.dll" -File |
    Select-Object -First 1

$CudaRuntime =
    Get-ChildItem $Audit -Recurse -Filter "cudart64_12.dll" -File |
    Select-Object -First 1

$VulkanServer =
    Get-ChildItem $Audit -Recurse -Filter "llama-server.exe" -File |
    Where-Object {
        $_.FullName -match "resources\\runtime\\vulkan\\current\\llama-server\.exe$"
    } |
    Select-Object -First 1

$VulkanBackend =
    Get-ChildItem $Audit -Recurse -Filter "ggml-vulkan.dll" -File |
    Select-Object -First 1

if (!$CudaServer) { throw "msi_payload_cuda_llama_server_missing" }
if (!$CudaBackend) { throw "msi_payload_ggml_cuda_missing" }
if (!$CudaBlas) { throw "msi_payload_cublas_missing" }
if (!$CudaBlasLt) { throw "msi_payload_cublaslt_missing" }
if (!$CudaRuntime) { throw "msi_payload_cudart_missing" }
if (!$VulkanServer) { throw "msi_payload_vulkan_llama_server_missing" }
if (!$VulkanBackend) { throw "msi_payload_ggml_vulkan_missing" }

Write-Host "MSI_CUDA_SERVER_PATH=$($CudaServer.FullName)"
Write-Host "MSI_CUDA_BACKEND_PATH=$($CudaBackend.FullName)"
Write-Host "MSI_VULKAN_SERVER_PATH=$($VulkanServer.FullName)"
Write-Host "MSI_VULKAN_BACKEND_PATH=$($VulkanBackend.FullName)"
Write-Host "MSI_CUDA_RUNTIME_PAYLOAD=PASS"
Write-Host "MSI_VULKAN_RUNTIME_PAYLOAD=PASS"


# WINDOWS_BACKGROUND_HELPER_PAYLOAD_V1
$Headless = Get-ChildItem $Audit -Recurse `
    -Filter 'edgeswarm-node-headless.exe' |
    Select-Object -First 1

$Supervisor = Get-ChildItem $Audit -Recurse `
    -Filter 'edgeswarm-node-supervisor.exe' |
    Select-Object -First 1

$TaskScript = Get-ChildItem $Audit -Recurse `
    -Filter 'supervisor-task.ps1' |
    Select-Object -First 1

if (!$Headless) {
    throw 'msi_payload_headless_missing'
}

if (!$Supervisor) {
    throw 'msi_payload_supervisor_missing'
}

if (!$TaskScript) {
    throw 'msi_payload_supervisor_task_script_missing'
}

Write-Host "MSI_HEADLESS_PATH=$($Headless.FullName)"
Write-Host "MSI_SUPERVISOR_PATH=$($Supervisor.FullName)"
Write-Host "MSI_SUPERVISOR_TASK_PATH=$($TaskScript.FullName)"
Write-Host 'MSI_HEADLESS_PAYLOAD=PASS'
Write-Host 'MSI_SUPERVISOR_PAYLOAD=PASS'
Write-Host 'MSI_SUPERVISOR_TASK_PAYLOAD=PASS'

# WINDOWS_UPDATER_RUNNER_PAYLOAD_V1
$UpdaterRunner = Get-ChildItem $Audit -Recurse `
    -Filter 'edgeswarm-updater-runner.exe' |
    Select-Object -First 1

if (!$UpdaterRunner) {
    throw 'msi_payload_updater_runner_missing'
}

Write-Host "MSI_UPDATER_RUNNER_PATH=$($UpdaterRunner.FullName)"
Write-Host 'MSI_UPDATER_RUNNER_PAYLOAD=PASS'
