param(
    [switch]$Build,
    [switch]$NoBuild,
    [switch]$Release,
    [switch]$Stop,
    [switch]$CheckOnly
)

$ErrorActionPreference = "Stop"

$RepoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
Set-Location -LiteralPath $RepoRoot

function Stop-FoundationProcesses {
    param([switch]$VerboseOutput)

    $procs = Get-Process -Name "kawari-*" -ErrorAction SilentlyContinue | Where-Object {
        $_.Path -and $_.Path.StartsWith($RepoRoot, [System.StringComparison]::OrdinalIgnoreCase)
    }

    if (-not $procs) {
        if ($VerboseOutput) {
            Write-Host "[Foundation] No running Foundation kawari-* processes found."
        }
        return
    }

    $count = @($procs).Count
    foreach ($p in $procs) {
        try {
            Stop-Process -Id $p.Id -Force -ErrorAction Stop
            try { $p.WaitForExit(3000) | Out-Null } catch {}
            if ($VerboseOutput) {
                Write-Host "[Foundation] Stopped $($p.ProcessName) (PID $($p.Id))"
            }
        }
        catch {
            Write-Warning "[Foundation] Failed to stop $($p.ProcessName) (PID $($p.Id)): $_"
        }
    }
    if (-not $VerboseOutput) {
        Write-Host "[Foundation] Stopped $count Foundation process(es)."
    }
}

Stop-FoundationProcesses -VerboseOutput:$Stop

if ($Stop) {
    exit 0
}

$Ports = 21057..21064
$Occupied = Get-NetTCPConnection -State Listen -ErrorAction SilentlyContinue |
    Where-Object { $_.LocalPort -in $Ports } |
    Sort-Object LocalPort

if ($Occupied) {
    Write-Host "[Foundation] Error: One or more required ports (21057-21064) are already in use by another process:" -ForegroundColor Red
    foreach ($conn in $Occupied) {
        $owner = Get-Process -Id $conn.OwningProcess -ErrorAction SilentlyContinue
        $ownerName = if ($owner) { $owner.ProcessName } else { "unknown" }
        Write-Host ("  Port {0} -> PID {1} ({2})" -f $conn.LocalPort, $conn.OwningProcess, $ownerName) -ForegroundColor Red
    }
    exit 1
}

$ConfigPath = Join-Path $RepoRoot "config.yaml"
if (-not (Test-Path -LiteralPath $ConfigPath)) {
    Write-Host "[Foundation] Error: Missing config.yaml at $ConfigPath" -ForegroundColor Red
    Write-Host "Create config.yaml in the repository root, for example:"
    Write-Host "  filesystem:"
    Write-Host '    game_path: "D:\\SquareEnix\\FINAL FANTASY XIV - A Realm Reborn\\game"'
    Write-Host "  web:"
    Write-Host '    server_name: "http://127.0.0.1:21062"'
    Write-Host "  world:"
    Write-Host '    server_name: "127.0.0.1"'
    exit 1
}

function Test-WorldHasOodle {
    param([string]$ExePath)

    if (-not (Test-Path -LiteralPath $ExePath)) {
        return $false
    }

    $text = [System.Text.Encoding]::GetEncoding(28591).GetString([System.IO.File]::ReadAllBytes($ExePath))
    return $text.Contains("oodle-network-shared") -and (-not $text.Contains("Something is trying to use Oodle"))
}

$ProfileName = if ($Release) { "release" } else { "debug" }
$ProfileDir = Join-Path $RepoRoot (Join-Path "target" $ProfileName)
$RunExe = Join-Path $ProfileDir "kawari-run.exe"
$WorldExe = Join-Path $ProfileDir "kawari-world.exe"
$LobbyExe = Join-Path $ProfileDir "kawari-lobby.exe"
$NavimeshExe = Join-Path $ProfileDir "kawari-navimesh.exe"

$RepoOodleDir = Join-Path $RepoRoot "oodle"
$RepoOodleDll = Join-Path $RepoOodleDir "oodle-network-shared.dll"
$OodleDll = Join-Path $ProfileDir "oodle-network-shared.dll"
$OodleCandidates = @(
    $RepoOodleDll,
    $OodleDll,
    (Join-Path $RepoRoot "target\debug\oodle-network-shared.dll"),
    (Join-Path $RepoRoot "target\release\oodle-network-shared.dll"),
    (Join-Path $RepoRoot "..\Kawari-Fork\target\debug\oodle-network-shared.dll"),
    (Join-Path $RepoRoot "..\Kawari-Fork\target\release\oodle-network-shared.dll")
)
$FoundOodleSource = $OodleCandidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
if (-not $FoundOodleSource) {
    Write-Host "[Foundation] Error: Missing oodle-network-shared.dll in $RepoOodleDir or $ProfileDir" -ForegroundColor Red
    Write-Host "Place oodle-network-shared.dll in $RepoOodleDir or $ProfileDir"
    exit 1
}

if (-not (Test-Path -LiteralPath $RepoOodleDll)) {
    New-Item -ItemType Directory -Path $RepoOodleDir -Force | Out-Null
    Copy-Item -LiteralPath $FoundOodleSource -Destination $RepoOodleDll -Force
    Write-Host "[Foundation] Populated $RepoOodleDll from $FoundOodleSource"
}

$MissingBinaries = (-not (Test-Path -LiteralPath $RunExe)) -or (-not (Test-Path -LiteralPath $WorldExe)) -or (-not (Test-Path -LiteralPath $LobbyExe))
$NeedsOodleRebuild = (Test-Path -LiteralPath $WorldExe) -and (-not (Test-WorldHasOodle -ExePath $WorldExe))
$ShouldBuild = $Build -or $MissingBinaries -or $NeedsOodleRebuild -or ((-not $NoBuild) -and (-not $CheckOnly))

if ($ShouldBuild) {
    if ($NeedsOodleRebuild -and -not $Build) {
        Write-Host "[Foundation] Detected kawari-world without Oodle feature; rebuilding with kawari/oodle..." -ForegroundColor Yellow
    }
    else {
        Write-Host "[Foundation] Building workspace (profile=$ProfileName, features=kawari/oodle)..." -ForegroundColor Cyan
    }
    $cargoArgs = @("build", "--workspace", "--features", "kawari/oodle")
    if (Test-Path -LiteralPath $NavimeshExe) {
        $cargoArgs += @("--exclude", "kawari-navimesh")
    }
    if ($Release) {
        $cargoArgs += "--release"
    }
    & cargo @cargoArgs
    if ($LASTEXITCODE -ne 0) {
        Write-Host "[Foundation] Build failed with exit code $LASTEXITCODE." -ForegroundColor Red
        exit $LASTEXITCODE
    }
}

if (-not (Test-Path -LiteralPath $OodleDll)) {
    New-Item -ItemType Directory -Path $ProfileDir -Force | Out-Null
    Copy-Item -LiteralPath $RepoOodleDll -Destination $OodleDll -Force
    Write-Host "[Foundation] Copied oodle-network-shared.dll to $OodleDll"
}

if ($CheckOnly) {
    Write-Host "Preflight OK" -ForegroundColor Green
    Write-Host "  Working Dir : $RepoRoot"
    Write-Host "  Executable  : $RunExe"
    Write-Host "  Oodle DLL   : $OodleDll"
    Write-Host "  Config      : $ConfigPath"
    exit 0
}

Write-Host "============================================================" -ForegroundColor Cyan
Write-Host " Foundation Housing Simulator Server" -ForegroundColor Cyan
Write-Host " Working Dir  : $RepoRoot"
Write-Host " Web Setup    : http://127.0.0.1:21062/setup"
Write-Host " Launcher URL : http://127.0.0.1:21062"
Write-Host " Housing API  : http://127.0.0.1:21064/api/v1/housing/status"
Write-Host "============================================================" -ForegroundColor Cyan

try {
    & $RunExe
    exit $LASTEXITCODE
}
finally {
    Stop-FoundationProcesses
}
