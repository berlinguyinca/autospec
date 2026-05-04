<# autospec-watchdog.ps1 — reclaim and nudge stalled autospec workers.

The monitor should call this every 12 iterations (or on similar cadence) to
detect stale `process-heartbeats/*.json` entries and:
  1) nudge the issue
  2) reclaim when stall time exceeds reclaim threshold

Environment overrides:
  AUTOSPEC_WATCHDOG_DIR: heartbeat directory (default: $HOME/.autospec/process-heartbeats)
  AUTOSPEC_WATCHDOG_REPO: override repo for gh calls (default: gh repo context)
  AUTOSPEC_WATCHDOG_STALE_SECS: stale threshold (default: 1800)
  AUTOSPEC_WATCHDOG_RECLAIM_SECS: reclaim threshold (default: 10800)
  AUTOSPEC_WATCHDOG_NUDGE_COOLDOWN_SECS: nudge cooldown (default: 900)
  AUTOSPEC_WATCHDOG_STATE_FILE: state file for nudge cooldown (default: $HOME/.autospec/watchdog-state.tsv)
#>

$ErrorActionPreference = "Stop"

function Get-EnvInt {
    param([string]$name, [int64]$default)
    $raw = [Environment]::GetEnvironmentVariable($name)
    if ([string]::IsNullOrWhiteSpace($raw)) { return $default }
    $value = 0
    if ([int64]::TryParse($raw, [ref]$value)) { return $value }
    return $default
}

function Exit-With-Message {
    param([string]$message, [int]$code = 0)
    Write-Output $message
    exit $code
}

if (-not (Get-Command gh -ErrorAction SilentlyContinue)) {
    Write-Error "[autospec-watchdog] ERROR: gh CLI not found"
    exit 1
}

$watchdogDir = [Environment]::GetEnvironmentVariable("AUTOSPEC_WATCHDOG_DIR")
if ([string]::IsNullOrWhiteSpace($watchdogDir)) {
    $watchdogDir = Join-Path $HOME ".autospec/process-heartbeats"
}
$watchdogRepo = [Environment]::GetEnvironmentVariable("AUTOSPEC_WATCHDOG_REPO")
if ([string]::IsNullOrWhiteSpace($watchdogRepo)) {
    $watchdogRepo = [Environment]::GetEnvironmentVariable("AUTOSPEC_REPO")
}
$staleSecs = Get-EnvInt "AUTOSPEC_WATCHDOG_STALE_SECS" 1800
$reclaimSecs = Get-EnvInt "AUTOSPEC_WATCHDOG_RECLAIM_SECS" 10800
$nudgeCooldownSecs = Get-EnvInt "AUTOSPEC_WATCHDOG_NUDGE_COOLDOWN_SECS" 900
$stateFile = [Environment]::GetEnvironmentVariable("AUTOSPEC_WATCHDOG_STATE_FILE")
if ([string]::IsNullOrWhiteSpace($stateFile)) {
    $stateFile = Join-Path $HOME ".autospec/watchdog-state.tsv"
}

if (-not (Test-Path $watchdogDir)) {
    Write-Output "service-watch: nudged=0 reclaimed=0 skipped=0"
    exit 0
}

$repoArgs = @()
if (-not [string]::IsNullOrWhiteSpace($watchdogRepo)) {
    $repoArgs = @("--repo", $watchdogRepo)
}

function GhIssueMeta {
    param([string]$IssueNumber)
    $args = @("issue", "view", $IssueNumber, "--json", "state,labels", "--jq", '.state + " " + ([.labels[].name] | join(","))') + $repoArgs
    & gh @args 2>$null
}

function Reclaim-Issue {
    param([string]$IssueNumber, [int64]$Age)
    $editArgs = @("issue", "edit", $IssueNumber, "--remove-label", "in-progress-by-bot", "--add-label", "auto-implement") + $repoArgs
    $commentArgs = @("issue", "comment", $IssueNumber, "--body", "autospec watchdog reclaimed this issue after $Age s of no check-in.") + $repoArgs
    & gh @editArgs 1>$null 2>$null
    & gh @commentArgs 1>$null 2>$null
}

function Nudge-Issue {
    param([string]$IssueNumber)
    $commentArgs = @("issue", "comment", $IssueNumber, "--body", "autospec watchdog: please check in; if stuck, post blocker and clear in-progress-by-bot.") + $repoArgs
    & gh @commentArgs 1>$null 2>$null
}

$lastNudge = @{}
if (Test-Path $stateFile) {
    Get-Content $stateFile | ForEach-Object {
        if ($_ -match '^\s*([0-9]+)\s+([0-9]+)\s*$') {
            $lastNudge[$matches[1]] = [int64]$matches[2]
        }
    }
}

$nudged = 0
$reclaimed = 0
$skipped = 0
$now = [DateTimeOffset]::UtcNow.ToUnixTimeSeconds()

Get-ChildItem -Path $watchdogDir -Filter '*.json' -File | ForEach-Object {
    $issue = $_.BaseName
    if ($issue -notmatch '^[0-9]+$') {
        $skipped++
        return
    }

    try {
        $heartbeat = Get-Content $_.FullName -Raw | ConvertFrom-Json -ErrorAction Stop
    } catch {
        $skipped++
        return
    }

    if ($null -eq $heartbeat.ts -or -not ($heartbeat.ts -is [int64])) {
        $skipped++
        return
    }

    $ts = [int64]$heartbeat.ts
    $age = [int64]($now - $ts)
    if ($age -lt 0) { $age = 0 }
    if ($age -lt $staleSecs) { return }

    $meta = GhIssueMeta -IssueNumber $issue
    if ([string]::IsNullOrWhiteSpace($meta)) {
        Remove-Item $_.FullName -Force
        $lastNudge.Remove($issue) | Out-Null
        $skipped++
        return
    }

    if ($meta -match '^([A-Za-z]+)\s*(.*)$') {
        $state = $matches[1]
        $labels = $matches[2]
    } else {
        $state = "UNKNOWN"
        $labels = ""
    }

    if ($state -ne "OPEN" -or (",${labels}," -notmatch ",in-progress-by-bot,") ) {
        Remove-Item $_.FullName -Force
        $lastNudge.Remove($issue) | Out-Null
        $skipped++
        return
    }

    if ($age -ge $reclaimSecs) {
        Reclaim-Issue -IssueNumber $issue -Age $age
        $reclaimed++
        Remove-Item $_.FullName -Force
        $lastNudge.Remove($issue) | Out-Null
        return
    }

    $shouldNudge = $true
    if ($lastNudge.ContainsKey($issue)) {
        $since = [int64]($now - $lastNudge[$issue])
        if ($since -lt $nudgeCooldownSecs) { $shouldNudge = $false }
    }

    if (-not $shouldNudge) { return }
    try {
        Nudge-Issue -IssueNumber $issue
        $nudged++
        $lastNudge[$issue] = $now
    } catch {
        $skipped++
    }
}

New-Item -ItemType Directory -Path (Split-Path $stateFile) -Force | Out-Null
if ($lastNudge.Count -eq 0) {
    Remove-Item $stateFile -ErrorAction SilentlyContinue
} else {
    $tmp = "$stateFile.$PID.tmp"
    $lastNudge.GetEnumerator() | ForEach-Object { "{0}`t{1}" -f $_.Key, $_.Value } | Set-Content -Path $tmp -NoNewline
    Move-Item $tmp $stateFile -Force
}

Write-Output ("service-watch: nudged={0} reclaimed={1} skipped={2}" -f $nudged, $reclaimed, $skipped)
