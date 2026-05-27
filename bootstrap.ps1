# bootstrap.ps1 - Windows-friendly bootstrap for the autospec suite.
#
# Usage:
#   irm https://raw.githubusercontent.com/berlinguyinca/autospec/main/bootstrap.ps1 | iex
#   ./bootstrap.ps1 --skill autospec --harness codex --update
#
# Honors:
#   AUTOSPEC_HOME       (default: $HOME\.autospec)
#   AUTOSPEC_REF        (default: main)
#   AUTOSPEC_REPO_URL   (default: official remote)

$ErrorActionPreference = "Stop"

function Write-Info {
    param([string]$Message)
    Write-Output "bootstrap: $Message"
}

function Write-Warn {
    param([string]$Message)
    Write-Warning "bootstrap: $Message"
}

function Invoke-Checked {
    param(
        [string]$CommandName,
        [string[]]$CommandArgs,
        [string]$ErrorMessage
    )
    & $CommandName @CommandArgs
    if ($LASTEXITCODE -ne 0) {
        throw $ErrorMessage
    }
}

function Install-CommandBestEffort {
    param(
        [string]$CommandName,
        [string]$WingetId,
        [string]$ChocoPackage,
        [string]$ScoopPackage
    )

    if (Get-Command $CommandName -ErrorAction SilentlyContinue) {
        return
    }

    if ((Get-Command winget -ErrorAction SilentlyContinue) -and -not [string]::IsNullOrWhiteSpace($WingetId)) {
        Write-Info "installing $CommandName via winget"
        & winget install --id $WingetId -e --accept-package-agreements --accept-source-agreements | Out-Null
        if (Get-Command $CommandName -ErrorAction SilentlyContinue) { return }
    }

    if ((Get-Command choco -ErrorAction SilentlyContinue) -and -not [string]::IsNullOrWhiteSpace($ChocoPackage)) {
        Write-Info "installing $CommandName via choco"
        & choco install -y $ChocoPackage | Out-Null
        if (Get-Command $CommandName -ErrorAction SilentlyContinue) { return }
    }

    if ((Get-Command scoop -ErrorAction SilentlyContinue) -and -not [string]::IsNullOrWhiteSpace($ScoopPackage)) {
        Write-Info "installing $CommandName via scoop"
        & scoop install $ScoopPackage | Out-Null
    }
}

function Find-Bash {
    $cmd = Get-Command bash -ErrorAction SilentlyContinue
    if ($cmd) { return $cmd.Source }

    $candidates = @(
        "$env:ProgramFiles\Git\bin\bash.exe",
        "${env:ProgramFiles(x86)}\Git\bin\bash.exe",
        "$env:LOCALAPPDATA\Programs\Git\bin\bash.exe"
    )
    foreach ($candidate in $candidates) {
        if ($candidate -and (Test-Path $candidate)) { return $candidate }
    }
    return $null
}

$autospecHome = [Environment]::GetEnvironmentVariable("AUTOSPEC_HOME")
if ([string]::IsNullOrWhiteSpace($autospecHome)) {
    $autospecHome = Join-Path $HOME ".autospec"
}
$autospecRef = [Environment]::GetEnvironmentVariable("AUTOSPEC_REF")
if ([string]::IsNullOrWhiteSpace($autospecRef)) {
    $autospecRef = "main"
}
$repoUrl = [Environment]::GetEnvironmentVariable("AUTOSPEC_REPO_URL")
if ([string]::IsNullOrWhiteSpace($repoUrl)) {
    $repoUrl = "https://github.com/berlinguyinca/autospec.git"
}
$repoDir = Join-Path $autospecHome "repo"

Install-CommandBestEffort "git" "Git.Git" "git" "git"
Install-CommandBestEffort "bash" "Git.Git" "git" "git"

if (-not (Get-Command git -ErrorAction SilentlyContinue)) {
    throw "git is required but was not found after best-effort installation."
}
$bash = Find-Bash
if ([string]::IsNullOrWhiteSpace($bash)) {
    throw "bash is required but was not found after best-effort Git installation."
}

New-Item -ItemType Directory -Force -Path $autospecHome | Out-Null

if (Test-Path (Join-Path $repoDir ".git")) {
    Write-Info "updating existing checkout at $repoDir (ref: $autospecRef)"
    Invoke-Checked -CommandName "git" -CommandArgs @("-C", $repoDir, "fetch", "--quiet", "origin", $autospecRef) -ErrorMessage "git fetch failed for $repoUrl ($autospecRef)"
    Invoke-Checked -CommandName "git" -CommandArgs @("-C", $repoDir, "checkout", "--quiet", $autospecRef) -ErrorMessage "git checkout failed for $autospecRef"
    & git -C $repoDir merge --ff-only --quiet "origin/$autospecRef"
    if ($LASTEXITCODE -ne 0) {
        & git -C $repoDir symbolic-ref --quiet HEAD | Out-Null
        if ($LASTEXITCODE -eq 0) {
            throw "could not fast-forward $repoDir to origin/$autospecRef. Resolve local changes and re-run."
        }
    }
} elseif (Test-Path $repoDir) {
    throw "$repoDir exists but is not a git checkout. Remove it or set AUTOSPEC_HOME and re-run."
} else {
    Write-Info "cloning $repoUrl to $repoDir (ref: $autospecRef)"
    & git clone --quiet --branch $autospecRef --depth 1 $repoUrl $repoDir
    if ($LASTEXITCODE -ne 0) {
        Write-Warn "shallow branch clone failed; retrying full clone"
        Invoke-Checked -CommandName "git" -CommandArgs @("clone", "--quiet", $repoUrl, $repoDir) -ErrorMessage "git clone failed for $repoUrl"
        Invoke-Checked -CommandName "git" -CommandArgs @("-C", $repoDir, "checkout", "--quiet", $autospecRef) -ErrorMessage "git checkout failed for $autospecRef"
    }
}

Write-Info "running $repoDir/install.sh $args"
Push-Location $repoDir
try {
    & $bash ./install.sh @args
    exit $LASTEXITCODE
} finally {
    Pop-Location
}
