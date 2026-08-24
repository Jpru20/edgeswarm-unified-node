param([string]$ConfigPath = '',[string]$TargetDir = '')
$ErrorActionPreference = 'Stop'
$Repo = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Set-Location $Repo
if ([string]::IsNullOrWhiteSpace($ConfigPath)) { $ConfigPath = Join-Path $Repo '.env.release.local' }
if (!(Test-Path -LiteralPath $ConfigPath)) { throw 'release_config_missing' }
$Config = @{}
Get-Content -LiteralPath $ConfigPath | ForEach-Object { $Line = $_.Trim(); if ($Line -and !$Line.StartsWith('#')) { $Parts = $Line -split '=',2; if ($Parts.Count -eq 2) { $Config[$Parts[0].Trim()] = $Parts[1].Trim() } } }
$Url = [string]$Config['EDGESWARM_DEFAULT_SUPABASE_URL']
$Key = [string]$Config['EDGESWARM_DEFAULT_SUPABASE_ANON_KEY']
if ([string]::IsNullOrWhiteSpace($Url) -or !$Url.StartsWith('https://')) { throw 'release_supabase_url_invalid' }
if ([string]::IsNullOrWhiteSpace($Key) -or $Key.Length -lt 50) { throw 'release_supabase_anon_key_invalid' }
$Head = (git rev-parse HEAD).Trim()
$Origin = (git rev-parse origin/main).Trim()
if ($Head -ne $Origin) { throw 'release_source_not_at_origin_main' }
$Dirty = @(git status --porcelain --untracked-files=no)
if ($Dirty.Count -gt 0) { throw 'release_worktree_not_clean' }
$env:EDGESWARM_DEFAULT_SUPABASE_URL = $Url
$env:EDGESWARM_DEFAULT_SUPABASE_ANON_KEY = $Key
Remove-Item Env:SUPABASE_URL -ErrorAction SilentlyContinue
Remove-Item Env:SUPABASE_ANON_KEY -ErrorAction SilentlyContinue
Remove-Item Env:EDGESWARM_SUPABASE_URL -ErrorAction SilentlyContinue
Remove-Item Env:EDGESWARM_SUPABASE_ANON_KEY -ErrorAction SilentlyContinue
if ([string]::IsNullOrWhiteSpace($TargetDir)) { $Short = (git rev-parse --short=7 HEAD).Trim(); $Stamp = Get-Date -Format 'yyyyMMdd-HHmmss'; $TargetDir = Join-Path $env:USERPROFILE "edgeswarm-release-build-$Short-$Stamp" }
$env:CARGO_TARGET_DIR = $TargetDir
Write-Host 'RELEASE_CONFIG_VALID=PASS'
Write-Host "SOURCE_COMMIT=$Head"
Write-Host "TARGET_DIR=$TargetDir"
npm.cmd run tauri build
if ($LASTEXITCODE -ne 0) { throw "tauri_build_failed_$LASTEXITCODE" }
$Exe = Join-Path $TargetDir 'release\edgeswarm-unified-node.exe'
$Msi = Get-ChildItem (Join-Path $TargetDir 'release\bundle\msi') -Filter '*.msi' | Select-Object -First 1
$Nsis = Get-ChildItem (Join-Path $TargetDir 'release\bundle\nsis') -Filter '*.exe' | Select-Object -First 1
if (!(Test-Path $Exe) -or !$Msi -or !$Nsis) { throw 'release_artifact_missing' }
$BinaryText = [Text.Encoding]::UTF8.GetString([IO.File]::ReadAllBytes($Exe))
if (!$BinaryText.Contains($Url)) { throw 'compiled_supabase_url_not_found' }
if (!$BinaryText.Contains($Key)) { throw 'compiled_supabase_anon_key_not_found' }
Write-Host 'COMPILED_CONFIG_VERIFIED=PASS'
foreach ($File in @($Exe,$Msi.FullName,$Nsis.FullName)) { $Item = Get-Item $File; $Hash = (Get-FileHash $File -Algorithm SHA256).Hash; $Sig = (Get-AuthenticodeSignature $File).Status; Write-Host "ARTIFACT=$($Item.FullName)"; Write-Host "BYTES=$($Item.Length)"; Write-Host "SHA256=$Hash"; Write-Host "SIGNATURE=$Sig" }
Write-Host 'RELEASE_BUILD_COMPLETE=PASS'
