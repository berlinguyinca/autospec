<# autospec-watchdog.ps1 — reclaim and nudge stalled autospec workers.

The monitor calls this before queue selection and on its regular service-watch
cadence to reconcile `process-heartbeats/*.json` entries:
  1) garbage-collect closed/orphaned heartbeats
  2) release stuck claimed heartbeats
  3) nudge stale active work
  4) reclaim when stall time exceeds reclaim threshold

Environment overrides:
  AUTOSPEC_WATCHDOG_DIR: heartbeat directory (default: $HOME/.autospec/process-heartbeats)
  AUTOSPEC_WATCHDOG_REPO: override repo for gh calls (default: gh repo context)
  AUTOSPEC_WATCHDOG_STALE_SECS: stale threshold (default: 1800)
  AUTOSPEC_WATCHDOG_RECLAIM_SECS: reclaim threshold (default: 10800)
  AUTOSPEC_WATCHDOG_CLAIMED_TIMEOUT_SECS: claimed timeout (default: 300)
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
$claimedTimeoutSecs = Get-EnvInt "AUTOSPEC_WATCHDOG_CLAIMED_TIMEOUT_SECS" 300
$nudgeCooldownSecs = Get-EnvInt "AUTOSPEC_WATCHDOG_NUDGE_COOLDOWN_SECS" 900
$stateFile = [Environment]::GetEnvironmentVariable("AUTOSPEC_WATCHDOG_STATE_FILE")
if ([string]::IsNullOrWhiteSpace($stateFile)) {
    $stateFile = Join-Path $HOME ".autospec/watchdog-state.tsv"
}

if (-not (Test-Path $watchdogDir)) {
    Write-Output "service-watch: nudged=0 reclaimed=0 claimed_released=0 garbage_collected=0 invalid_schema=0 skipped=0"
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

function Release-Claim {
    param([string]$IssueNumber, [int64]$Age)
    $editArgs = @("issue", "edit", $IssueNumber, "--remove-label", "in-progress-by-bot", "--add-label", "auto-implement") + $repoArgs
    $commentArgs = @("issue", "comment", $IssueNumber, "--body", "autospec watchdog released this claim after $Age s without reaching worktree_ready.") + $repoArgs
    & gh @editArgs 1>$null 2>$null
    & gh @commentArgs 1>$null 2>$null
}

function Nudge-Issue {
    param([string]$IssueNumber)
    $commentArgs = @("issue", "comment", $IssueNumber, "--body", "autospec watchdog: please check in; if stuck, post blocker and clear in-progress-by-bot.") + $repoArgs
    & gh @commentArgs 1>$null 2>$null
}

function Get-HeartbeatValue {
    param([object]$Heartbeat, [string]$Name)
    $prop = $Heartbeat.PSObject.Properties[$Name]
    if ($null -eq $prop -or $null -eq $prop.Value) { return "" }
    return [string]$prop.Value
}

function Test-HeartbeatSchema {
    param([object]$Heartbeat, [string]$IssueNumber)
    $step = Get-HeartbeatValue $Heartbeat "step"
    $issueValue = Get-HeartbeatValue $Heartbeat "issue"
    $tsRaw = Get-HeartbeatValue $Heartbeat "ts"
    $tsValue = 0L
    $validSteps = @("claimed", "worktree_ready", "tests_started", "tests_passed", "pr_created", "smoke_retry", "reviewed", "merged", "failed")

    if ($issueValue -ne $IssueNumber) { return $false }
    if (-not [int64]::TryParse($tsRaw, [ref]$tsValue)) { return $false }
    if ($validSteps -notcontains $step) { return $false }
    return $true
}

function Normalize-Heartbeat {
    param([object]$Heartbeat, [string]$IssueNumber, [string]$Path)
    $tsRaw = Get-HeartbeatValue $Heartbeat "ts"
    $tsValue = 0L
    [void][int64]::TryParse($tsRaw, [ref]$tsValue)
    $normalized = [ordered]@{
        issue = $IssueNumber
        branch = Get-HeartbeatValue $Heartbeat "branch"
        step = Get-HeartbeatValue $Heartbeat "step"
        ts = $tsValue
        pr = Get-HeartbeatValue $Heartbeat "pr"
        repo = Get-HeartbeatValue $Heartbeat "repo"
    }
    $normalized | ConvertTo-Json -Compress | Set-Content -Path $Path -NoNewline
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
$claimedReleased = 0
$garbageCollected = 0
$invalidSchema = 0
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
        Remove-Item $_.FullName -Force
        $invalidSchema++
        return
    }

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
        $garbageCollected++
        return
    }

    if (-not (Test-HeartbeatSchema -Heartbeat $heartbeat -IssueNumber $issue)) {
        Remove-Item $_.FullName -Force
        $lastNudge.Remove($issue) | Out-Null
        $invalidSchema++
        return
    }

    Normalize-Heartbeat -Heartbeat $heartbeat -IssueNumber $issue -Path $_.FullName

    $ts = [int64](Get-HeartbeatValue $heartbeat "ts")
    $age = [int64]($now - $ts)
    if ($age -lt 0) { $age = 0 }

    if ((Get-HeartbeatValue $heartbeat "step") -eq "claimed" -and $age -ge $claimedTimeoutSecs) {
        Release-Claim -IssueNumber $issue -Age $age
        $claimedReleased++
        Remove-Item $_.FullName -Force
        $lastNudge.Remove($issue) | Out-Null
        return
    }

    if ($age -lt $staleSecs) { return }

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

Write-Output ("service-watch: nudged={0} reclaimed={1} claimed_released={2} garbage_collected={3} invalid_schema={4} skipped={5}" -f $nudged, $reclaimed, $claimedReleased, $garbageCollected, $invalidSchema, $skipped)
