param(
    [ValidateSet("Probe", "Install", "Remove", "PauseForUpdate", "RunUpdater")]
    [string]$Mode = "Probe",

    [string]$SupervisorPath = "",

    [string]$AppPath = "",

    [string]$UpdaterEndpoint = ""
)

$ErrorActionPreference = "Stop"
$TaskName = "EdgeSwarm Node Supervisor"
$UpdaterTaskName = "EdgeSwarm Node Updater"
$Identity = [System.Security.Principal.WindowsIdentity]::GetCurrent()

if (-not $Identity.User) {
    throw "current_user_sid_missing"
}

$Sid = $Identity.User.Value
$UserName = $Identity.Name


# SCHEDULED_UPDATER_RUNNER_V2
#
# Keep the Scheduled Task process alive until the signed Tauri updater
# and the NSIS installation have actually replaced the installed app.
if ($Mode -eq "RunUpdater") {
    if ([string]::IsNullOrWhiteSpace($AppPath) -or -not (Test-Path -LiteralPath $AppPath)) {
        Write-Error "updater_app_path_missing"
        exit 20
    }

    $InstallDir = Split-Path -Parent $AppPath
    $LogPath = Join-Path $InstallDir "updater-task.log"

    # UPDATE_LIFECYCLE_FILE_V1
    $LifecyclePath =
        Join-Path $InstallDir "update-lifecycle.json"

    function Write-UpdaterLog {
        param([string]$Message)

        $Timestamp = [DateTimeOffset]::UtcNow.ToString("o")
        Add-Content -LiteralPath $LogPath -Value "$Timestamp $Message" -Encoding UTF8
    }

    function Write-UpdateLifecycle {
        param([string]$Phase)

        $NowMs =
            [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()

        # UPDATE_LIFECYCLE_UTF8_NO_BOM_V1
        $Payload =
            @{
                schemaVersion = 1
                phase = $Phase
                fromVersion = $BeforeVersion
                toVersion = $TargetVersion
                startedAtUnixMs = $StartedAtMs
                updatedAtUnixMs = $NowMs
            } |
            ConvertTo-Json

        $Utf8NoBom =
            New-Object System.Text.UTF8Encoding($false)

        [IO.File]::WriteAllText(
            $LifecyclePath,
            $Payload,
            $Utf8NoBom
        )
    }

    # UPDATE_FAILURE_PROVIDER_RECOVERY_V1
    function Restore-ProviderAfterUpdateFailure {
        $PauseLockPath =
            Join-Path $InstallDir "update-pause.lock"

        Remove-Item `
            -LiteralPath $PauseLockPath `
            -Force `
            -ErrorAction SilentlyContinue

        Write-UpdaterLog "UPDATE_PAUSE_LOCK_CLEARED_ON_FAILURE=true"

        try {
            Start-ScheduledTask `
                -TaskName $TaskName `
                -ErrorAction Stop

            Write-UpdaterLog "PROVIDER_RESTART_AFTER_UPDATE_FAILURE=true"
        } catch {
            Write-UpdaterLog `
                "ERROR=provider_restart_after_update_failure_failed message=$($_.Exception.Message)"
        }
    }

    $BeforeVersion = (Get-Item -LiteralPath $AppPath).VersionInfo.ProductVersion

    if ([string]::IsNullOrWhiteSpace($BeforeVersion)) {
        Write-UpdaterLog "ERROR=current_version_missing"
        exit 21
    }

    if ([string]::IsNullOrWhiteSpace($UpdaterEndpoint)) {
        $ManifestUrl = "https://api.edgeswarm.io/node/desktop-update/windows/x86_64/$BeforeVersion"
    } else {
        $ManifestUrl = $UpdaterEndpoint
    }

    Write-UpdaterLog "CHECK version=$BeforeVersion url=$ManifestUrl"

    try {
        $Response = Invoke-WebRequest -UseBasicParsing -Uri $ManifestUrl -Method Get -TimeoutSec 30
    } catch {
        Write-UpdaterLog "ERROR=manifest_request_failed message=$($_.Exception.Message)"
        exit 22
    }

    if ($Response.StatusCode -eq 204) {
        Write-UpdaterLog "NO_UPDATE=true"
        exit 0
    }

    if ($Response.StatusCode -ne 200) {
        Write-UpdaterLog "ERROR=manifest_http_$($Response.StatusCode)"
        exit 23
    }

    try {
        $Manifest = $Response.Content | ConvertFrom-Json
    } catch {
        Write-UpdaterLog "ERROR=manifest_json_invalid"
        exit 24
    }

    $TargetVersion = [string]$Manifest.version

    if ([string]::IsNullOrWhiteSpace($TargetVersion)) {
        Write-UpdaterLog "ERROR=target_version_missing"
        exit 25
    }

    $StartedAtMs =
        [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()

    Write-UpdateLifecycle "preparing"

    Write-UpdaterLog "UPDATE_AVAILABLE from=$BeforeVersion to=$TargetVersion"

    # The provider heartbeat runs every ~15 seconds. Give it one
    # guaranteed cycle to publish the update state before any provider
    # process is paused.
    Start-Sleep -Seconds 18

    $UpdaterArguments = @("--background-update-check")

    if (-not [string]::IsNullOrWhiteSpace($UpdaterEndpoint)) {
        $UpdaterArguments += "--background-update-endpoint"
        $UpdaterArguments += $UpdaterEndpoint
    }

    # UPDATER_GUI_CLOSE_V1
    #
    # The update runner is external to the Tauri application, so it can
    # close any existing interactive copy before launching the dedicated
    # background updater process. This prevents the installed EXE from
    # remaining locked while NSIS tries to replace it.
    $ExpectedApp =
        [IO.Path]::GetFullPath($AppPath)

    # UPDATER_GUI_FAIL_CLOSED_V2
    #
    # A Limited scheduled task cannot inspect ExecutablePath for an
    # elevated copy of the GUI. Never interpret a hidden path as
    # "no GUI running" because NSIS would then hit a file lock.
    $AllExistingApps =
        @(
            Get-CimInstance Win32_Process `
                -Filter "Name='edgeswarm-unified-node.exe'" `
                -ErrorAction SilentlyContinue
        )

    $InaccessibleApps =
        @(
            $AllExistingApps |
            Where-Object {
                [string]::IsNullOrWhiteSpace(
                    [string]$_.ExecutablePath
                )
            }
        )

    if ($InaccessibleApps.Count -gt 0) {
        Write-UpdateLifecycle "failed"

        foreach ($InaccessibleApp in $InaccessibleApps) {
            Write-UpdaterLog `
                "ERROR=gui_process_path_inaccessible pid=$($InaccessibleApp.ProcessId)"
        }

        Write-UpdaterLog `
            "ERROR=gui_close_privilege_mismatch"

        exit 29
    }

    $ExistingApps =
        @(
            $AllExistingApps |
            Where-Object {
                ([string]$_.ExecutablePath).Equals(
                    $ExpectedApp,
                    [StringComparison]::OrdinalIgnoreCase
                )
            }
        )

    foreach ($ExistingApp in $ExistingApps) {
        Stop-Process `
            -Id $ExistingApp.ProcessId `
            -Force `
            -ErrorAction SilentlyContinue

        Write-UpdaterLog `
            "GUI_PROCESS_STOPPED_PID=$($ExistingApp.ProcessId)"
    }

    $GuiDeadline =
        (Get-Date).AddSeconds(10)

    do {
        $AllRemainingApps =
            @(
                Get-CimInstance Win32_Process `
                    -Filter "Name='edgeswarm-unified-node.exe'" `
                    -ErrorAction SilentlyContinue
            )

        $InaccessibleRemainingApps =
            @(
                $AllRemainingApps |
                Where-Object {
                    [string]::IsNullOrWhiteSpace(
                        [string]$_.ExecutablePath
                    )
                }
            )

        $RemainingApps =
            @(
                $AllRemainingApps |
                Where-Object {
                    $_.ExecutablePath -and
                    ([string]$_.ExecutablePath).Equals(
                        $ExpectedApp,
                        [StringComparison]::OrdinalIgnoreCase
                    )
                }
            )

        if (
            $RemainingApps.Count -eq 0 -and
            $InaccessibleRemainingApps.Count -eq 0
        ) {
            break
        }

        Start-Sleep -Milliseconds 250
    }
    while ((Get-Date) -lt $GuiDeadline)

    if ($InaccessibleRemainingApps.Count -gt 0) {
        Write-UpdateLifecycle "failed"

        foreach ($InaccessibleApp in $InaccessibleRemainingApps) {
            Write-UpdaterLog `
                "ERROR=gui_process_path_inaccessible_after_stop pid=$($InaccessibleApp.ProcessId)"
        }

        Write-UpdaterLog `
            "ERROR=gui_close_privilege_mismatch"

        exit 29
    }

    if ($RemainingApps.Count -gt 0) {
        Write-UpdateLifecycle "failed"
        Write-UpdaterLog "ERROR=gui_process_stop_timeout"
        exit 29
    }

    Write-UpdateLifecycle "updating"

    $StdoutPath =
        Join-Path $InstallDir "updater-background.stdout.log"

    $StderrPath =
        Join-Path $InstallDir "updater-background.stderr.log"

    Remove-Item `
        $StdoutPath,$StderrPath `
        -Force `
        -ErrorAction SilentlyContinue

    Write-UpdaterLog "UPDATER_PROCESS_START=true"

    try {
        $Process =
            Start-Process `
                -FilePath $AppPath `
                -ArgumentList $UpdaterArguments `
                -WorkingDirectory $InstallDir `
                -RedirectStandardOutput $StdoutPath `
                -RedirectStandardError $StderrPath `
                -PassThru `
                -Wait
    } catch {
        Write-UpdateLifecycle "failed"

        Write-UpdaterLog `
            "ERROR=updater_process_launch_failed message=$($_.Exception.Message)"

        exit 26
    }

    Write-UpdaterLog `
        "UPDATER_PROCESS_EXIT=$($Process.ExitCode)"

    if (Test-Path -LiteralPath $StdoutPath) {
        Get-Content -LiteralPath $StdoutPath |
        ForEach-Object {
            Write-UpdaterLog "BACKGROUND_STDOUT=$_"
        }
    }

    if (Test-Path -LiteralPath $StderrPath) {
        Get-Content -LiteralPath $StderrPath |
        ForEach-Object {
            Write-UpdaterLog "BACKGROUND_STDERR=$_"
        }
    }

    # BACKGROUND_UPDATE_ERROR_FAIL_FAST_V2
    #
    # Explicit updater errors remain authoritative failures.
    # A missing final success marker is NOT a failure on Windows:
    # NSIS may replace/terminate the updating executable before its
    # final stdout marker can be flushed. The external runner verifies
    # success authoritatively by observing TargetVersion on disk.
    $BackgroundError =
        if (Test-Path -LiteralPath $StderrPath) {
            Select-String `
                -LiteralPath $StderrPath `
                -Pattern '^BACKGROUND_UPDATE_ERROR=' `
                -ErrorAction SilentlyContinue |
            Select-Object -First 1
        } else {
            $null
        }

    $BackgroundSuccess =
        if (Test-Path -LiteralPath $StdoutPath) {
            Select-String `
                -LiteralPath $StdoutPath `
                -Pattern '^BACKGROUND_UPDATE_RESULT=success$' `
                -ErrorAction SilentlyContinue |
            Select-Object -First 1
        } else {
            $null
        }

    if (
        $Process.ExitCode -ne 0 -or
        $BackgroundError
    ) {
        Write-UpdateLifecycle "failed"

        if ($BackgroundError) {
            Write-UpdaterLog `
                "ERROR=background_updater_reported_failure detail=$($BackgroundError.Line)"
        } else {
            Write-UpdaterLog `
                "ERROR=updater_process_failed exit=$($Process.ExitCode)"
        }

        Restore-ProviderAfterUpdateFailure
        exit 27
    }

    Write-UpdaterLog `
        "BACKGROUND_SUCCESS_MARKER_PRESENT=$([bool]$BackgroundSuccess)"

    # TARGET_VERSION_EXTERNAL_VERIFICATION_V1
    #
    # From this point forward, installation success is determined by
    # the signed target version actually replacing AppPath.
    Write-UpdateLifecycle "installing"

    $Deadline = (Get-Date).AddMinutes(5)

    while ((Get-Date) -lt $Deadline) {
        Start-Sleep -Seconds 2

        if (-not (Test-Path -LiteralPath $AppPath)) {
            continue
        }

        $CurrentVersion = (Get-Item -LiteralPath $AppPath).VersionInfo.ProductVersion

        if ($CurrentVersion -eq $TargetVersion) {
            Write-UpdateLifecycle "restarting"

            Write-UpdaterLog "UPDATE_COMPLETE version=$CurrentVersion"

            Start-Sleep -Seconds 2

            Remove-Item `
                -LiteralPath $LifecyclePath `
                -Force `
                -ErrorAction SilentlyContinue

            exit 0
        }
    }

    Write-UpdateLifecycle "failed"

    Write-UpdaterLog "ERROR=install_timeout target=$TargetVersion"

    Restore-ProviderAfterUpdateFailure
    exit 28
}

Write-Host "TASK_NAME=$TaskName"
Write-Host "TASK_USER=$UserName"
Write-Host "TASK_USER_SID=$Sid"
Write-Host "TASK_LOGON_TYPE=Interactive"
Write-Host "TASK_RUN_LEVEL=Limited"

if ($Mode -in @("Remove", "PauseForUpdate")) {
    # UPDATE_PAUSE_LOCK_CREATE_V1
    if ($Mode -eq "PauseForUpdate") {
        if ([string]::IsNullOrWhiteSpace($AppPath)) {
            throw "update_pause_app_path_required"
        }

        $PauseAppPath =
            [IO.Path]::GetFullPath($AppPath)

        $PauseInstallDir =
            Split-Path -Parent $PauseAppPath

        $UpdatePauseLockPath =
            Join-Path `
                $PauseInstallDir `
                "update-pause.lock"

        $PausePayload =
            "createdAtUnixMs=$([DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds())"

        $PauseUtf8NoBom =
            New-Object System.Text.UTF8Encoding($false)

        [IO.File]::WriteAllText(
            $UpdatePauseLockPath,
            $PausePayload,
            $PauseUtf8NoBom
        )

        Write-Host "UPDATE_PAUSE_LOCK_CREATED=true"
    }

    if ($Mode -eq "Remove") {
        $updaterTask = Get-ScheduledTask `
            -TaskName $UpdaterTaskName `
            -ErrorAction SilentlyContinue

        if ($updaterTask) {
            Stop-ScheduledTask `
                -TaskName $UpdaterTaskName `
                -ErrorAction SilentlyContinue

            Unregister-ScheduledTask `
                -TaskName $UpdaterTaskName `
                -Confirm:$false
        }

        Write-Host "UPDATER_TASK_REMOVED=true"
    } else {
        # The updater task is the process currently performing
        # the signed update. Preserve it during update-time pause.
        Write-Host "UPDATER_TASK_PRESERVED_FOR_UPDATE=true"
    }

    $existing = Get-ScheduledTask `
        -TaskName $TaskName `
        -ErrorAction SilentlyContinue

    if ($existing) {
        Stop-ScheduledTask `
            -TaskName $TaskName `
            -ErrorAction SilentlyContinue

        if ($Mode -eq "Remove") {
            Unregister-ScheduledTask `
                -TaskName $TaskName `
                -Confirm:$false
        } else {
            # SUPERVISOR_TASK_PRESERVE_DURING_UPDATE_V1
            #
            # The Scheduled Updater runs as the normal interactive user.
            # It can stop the supervisor task but may not own the task ACL
            # required to unregister it. An application update does not need
            # the registration removed; only the running provider stack must
            # stop before installer file replacement.
            Write-Host `
                "SUPERVISOR_TASK_PRESERVED_FOR_UPDATE=true"
        }
    }

    # WINDOWS_SUPERVISOR_UNINSTALL_CLEANUP_V1
    #
    # If a supervisor instance is still alive, terminate ONLY an
    # instance whose executable path exactly matches the installed
    # supervisor supplied by the uninstaller. Its Windows Job Object
    # then cleans up headless + llama descendants.
    if (-not [string]::IsNullOrWhiteSpace($SupervisorPath)) {
        $ExpectedSupervisor =
            [IO.Path]::GetFullPath($SupervisorPath)

        $supervisors =
            Get-CimInstance Win32_Process `
                -Filter "Name='edgeswarm-node-supervisor.exe'" `
                -ErrorAction SilentlyContinue

        foreach ($process in $supervisors) {
            $actual =
                [string]$process.ExecutablePath

            if (
                $actual -and
                $actual.Equals(
                    $ExpectedSupervisor,
                    [StringComparison]::OrdinalIgnoreCase
                )
            ) {
                Stop-Process `
                    -Id $process.ProcessId `
                    -Force `
                    -ErrorAction SilentlyContinue

                Write-Host `
                    "SUPERVISOR_PROCESS_STOPPED_PID=$($process.ProcessId)"
            }
        }

        $deadline =
            (Get-Date).AddSeconds(10)

        do {
            $remaining =
                Get-CimInstance Win32_Process `
                    -Filter "Name='edgeswarm-node-supervisor.exe'" `
                    -ErrorAction SilentlyContinue |
                Where-Object {
                    $_.ExecutablePath -and
                    ([string]$_.ExecutablePath).Equals(
                        $ExpectedSupervisor,
                        [StringComparison]::OrdinalIgnoreCase
                    )
                }

            if (-not $remaining) {
                break
            }

            Start-Sleep -Milliseconds 250
        }
        while ((Get-Date) -lt $deadline)

        if ($remaining) {
            throw "supervisor_process_stop_timeout"
        }
    }

    if ($Mode -eq "Remove") {
        Write-Host "SUPERVISOR_TASK_REMOVED=true"
        Write-Host "SUPERVISOR_UNINSTALL_CLEANUP=true"
    } else {
        Write-Host "SUPERVISOR_UPDATE_PAUSE_COMPLETE=true"
    }

    exit 0
}

if ([string]::IsNullOrWhiteSpace($SupervisorPath)) {
    throw "supervisor_path_required"
}

$SupervisorPath = [IO.Path]::GetFullPath($SupervisorPath)

if ([string]::IsNullOrWhiteSpace($AppPath)) {
    $AppPath =
        Join-Path `
            (Split-Path $SupervisorPath -Parent) `
            "edgeswarm-unified-node.exe"
}

$AppPath = [IO.Path]::GetFullPath($AppPath)

if (-not (Test-Path -LiteralPath $AppPath -PathType Leaf)) {
    throw "updater_app_binary_missing"
}

if (-not (Test-Path -LiteralPath $SupervisorPath -PathType Leaf)) {
    throw "supervisor_binary_missing"
}

if ($Mode -eq "Probe") {
    $existing = Get-ScheduledTask -TaskName $TaskName -ErrorAction SilentlyContinue

    Write-Host "SUPERVISOR_PATH=$SupervisorPath"
    Write-Host "TASK_ALREADY_REGISTERED=$([bool]$existing)"
    Write-Host "SUPERVISOR_TASK_PROBE_PASS=true"
    exit 0
}

$action = New-ScheduledTaskAction `
    -Execute $SupervisorPath

$logonTrigger = New-ScheduledTaskTrigger `
    -AtLogOn `
    -User $UserName

# OUTER_WATCHDOG_MINUTE_TRIGGER_V1
#
# Task Scheduler does not reliably classify every externally
# terminated process as a restartable "failure". This independent
# trigger attempts to start the supervisor every minute.
# MultipleInstances=IgnoreNew makes the attempt a no-op while the
# existing supervisor is healthy.
$watchdogTrigger = New-ScheduledTaskTrigger `
    -Once `
    -At ((Get-Date).AddMinutes(1)) `
    -RepetitionInterval (New-TimeSpan -Minutes 1)

$triggers = @(
    $logonTrigger,
    $watchdogTrigger
)

$principal = New-ScheduledTaskPrincipal `
    -UserId $Sid `
    -LogonType Interactive `
    -RunLevel Limited

$settings = New-ScheduledTaskSettingsSet `
    -AllowStartIfOnBatteries `
    -DontStopIfGoingOnBatteries `
    -StartWhenAvailable `
    -Hidden `
    -MultipleInstances IgnoreNew `
    -RestartCount 999 `
    -RestartInterval (New-TimeSpan -Minutes 1) `
    -ExecutionTimeLimit ([TimeSpan]::Zero)

# INPLACE_TASK_REGISTRATION_PRESERVE_V1
$existingSupervisorTask =
    Get-ScheduledTask `
        -TaskName $TaskName `
        -ErrorAction SilentlyContinue

if ($existingSupervisorTask) {
    Write-Host "SUPERVISOR_TASK_PRESERVED_EXISTING=true"
} else {
    Register-ScheduledTask `
        -TaskName $TaskName `
        -Action $action `
        -Trigger $triggers `
        -Principal $principal `
        -Settings $settings `
        -Force | Out-Null

    Write-Host "SUPERVISOR_TASK_REGISTERED=true"
}

if (-not (Get-Process "edgeswarm-node-supervisor" -ErrorAction SilentlyContinue)) {
    Start-ScheduledTask -TaskName $TaskName
}
$updaterPowerShell = "$env:SystemRoot\\System32\\WindowsPowerShell\\v1.0\\powershell.exe"
$updaterWorkingDirectory = Split-Path -Parent $AppPath
$updaterArguments = "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File `"$PSCommandPath`" -Mode RunUpdater -AppPath `"$AppPath`""

$updaterAction = New-ScheduledTaskAction -Execute $updaterPowerShell -Argument $updaterArguments -WorkingDirectory $updaterWorkingDirectory

$updaterLogonTrigger =
    New-ScheduledTaskTrigger `
        -AtLogOn `
        -User $UserName

$updaterHourlyTrigger =
    New-ScheduledTaskTrigger `
        -Once `
        -At ((Get-Date).AddMinutes(2)) `
        -RepetitionInterval (New-TimeSpan -Hours 1)

$updaterSettings =
    New-ScheduledTaskSettingsSet `
        -AllowStartIfOnBatteries `
        -DontStopIfGoingOnBatteries `
        -StartWhenAvailable `
        -Hidden `
        -MultipleInstances IgnoreNew `
        -ExecutionTimeLimit (New-TimeSpan -Minutes 30)

$existingUpdaterTask =
    Get-ScheduledTask `
        -TaskName $UpdaterTaskName `
        -ErrorAction SilentlyContinue

if ($existingUpdaterTask) {
    Write-Host "UPDATER_TASK_PRESERVED_EXISTING=true"
} else {
    Register-ScheduledTask `
        -TaskName $UpdaterTaskName `
        -Action $updaterAction `
        -Trigger @(
            $updaterLogonTrigger,
            $updaterHourlyTrigger
        ) `
        -Principal $principal `
        -Settings $updaterSettings `
        -Force | Out-Null

    Write-Host "UPDATER_TASK_REGISTERED=true"
    Write-Host "UPDATER_TASK_INTERVAL=PT1H"
}

# UPDATE_PAUSE_LOCK_RELEASE_V1
$UpdatePauseLockPath =
    Join-Path `
        (Split-Path -Parent $AppPath) `
        "update-pause.lock"

Remove-Item `
    -LiteralPath $UpdatePauseLockPath `
    -Force `
    -ErrorAction SilentlyContinue

Write-Host "UPDATE_PAUSE_LOCK_CLEARED=true"

if (-not (
    Get-Process `
        "edgeswarm-node-supervisor" `
        -ErrorAction SilentlyContinue
)) {
    Start-ScheduledTask `
        -TaskName $TaskName

    Write-Host "SUPERVISOR_TASK_STARTED_AFTER_INSTALL=true"
}

Write-Host "SUPERVISOR_TASK_RESTART_ON_FAILURE=true"
Write-Host "SUPERVISOR_TASK_PASSWORD_STORED=false"
