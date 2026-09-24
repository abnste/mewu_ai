# SPDX-License-Identifier: MPL-2.0
[CmdletBinding()]
param(
    [ValidateSet('Repository', 'Publish')][string]$Mode = 'Repository',
    [string]$PublishDirectory,
    [ValidatePattern('^[A-Za-z0-9][A-Za-z0-9._/@^~:-]*$')][string]$RevisionRange = 'HEAD'
)
$ErrorActionPreference = 'Stop'
$repositoryRoot = Split-Path $PSScriptRoot -Parent

function Test-PrivatePath([string]$Path) {
    $normalized = $Path.Replace('\', '/').TrimStart('/')
    return (
        $normalized -match '(?i)(^|/)(agents?|claude|gemini)\.md$' -or
        $normalized -match '(?i)(^|/)(\.private|\.codex-build|promo-video|Credentials|History|Clipboard|Logs?|Temp|Screenshots|Recordings)(/|$)' -or
        $normalized -match '(?i)(^|/)(settings\.json|secrets\.json|\.env(?:\..*)?|id_rsa|id_ed25519)$' -or
        $normalized -match '(?i)\.(pdb|log|jsonl|tmp|pem|key|pfx|p12|p8|dmp|dump|sqlite3?)$' -or
        $normalized -match '(?i)^docs/(codex-work-integration|workbuddy-integration|interaction-latency-.*|optimization-review-.*|teaching-evaluation-.*|teaching-mode-.*|translation-fix-.*|window-issues-.*|patent-research-.*)\.md$'
    )
}

if ($Mode -eq 'Publish') {
    if (-not $PublishDirectory) { throw 'PublishDirectory is required.' }
    $payloadRoot = (Resolve-Path -LiteralPath $PublishDirectory).Path
    $forbidden = @(Get-ChildItem -LiteralPath $payloadRoot -Recurse -File -Force | Where-Object {
        $relative = [IO.Path]::GetRelativePath($payloadRoot, $_.FullName)
        (Test-PrivatePath $relative) -or $_.Extension -match '(?i)^\.(png|jpe?g|gif|mp4|webm|avi|mkv|mov|wav|mp3)$'
    })
    if ($forbidden.Count) { throw "Publish privacy check rejected $($forbidden.Count) private/development file(s). File contents are not logged." }
    Write-Output 'Publish privacy check passed.'
    exit 0
}

Push-Location $repositoryRoot
try {
    $tracked = @(git -c core.quotePath=false ls-files)
    if ($LASTEXITCODE -ne 0) { throw 'Unable to enumerate tracked files.' }
    $forbidden = @($tracked | Where-Object { $_ -notmatch '(^|/)\.env\.example$' -and (Test-PrivatePath $_) })
    if ($forbidden.Count) { throw "Repository privacy check rejected $($forbidden.Count) tracked private/development file(s)." }

    # Also reject previously added private files, even if later deleted.
    $historyPaths = @(git -c core.quotePath=false log --format= --name-only --diff-filter=ACMR $RevisionRange)
    if ($LASTEXITCODE -ne 0) { throw 'Unable to inspect outgoing history.' }
    $privateHistory = @($historyPaths | Where-Object { $_ -and $_ -notmatch '(^|/)\.env\.example$' -and (Test-PrivatePath $_) })
    if ($privateHistory.Count) { throw 'Private files remain in the selected history. Keep original backups local.' }

    $emails = @(git log --format='%ae%n%ce' $RevisionRange)
    if ($LASTEXITCODE -ne 0) { throw 'Unable to inspect commit identities.' }
    if (@($emails | Where-Object { $_ -notmatch '^[^@\s]+@users\.noreply\.github\.com$|^noreply@github\.com$' }).Count) {
        throw 'Commit metadata contains a non-private email. Use the GitHub privacy email for the corresponding author; preserve author names.'
    }

    $toolDirectory = Join-Path $repositoryRoot '.codex-build/privacy-tools'
    New-Item -ItemType Directory -Path $toolDirectory -Force | Out-Null
    $archive = Join-Path $toolDirectory 'gitleaks-8.30.1.zip'
    $expectedHash = 'd29144deff3a68aa93ced33dddf84b7fdc26070add4aa0f4513094c8332afc4e'
    if (-not (Test-Path -LiteralPath $archive)) {
        $download = Join-Path $toolDirectory ([guid]::NewGuid().ToString('N') + '.tmp')
        try {
            Invoke-WebRequest 'https://github.com/gitleaks/gitleaks/releases/download/v8.30.1/gitleaks_8.30.1_windows_x64.zip' -OutFile $download -TimeoutSec 120
            if ((Get-FileHash -LiteralPath $download -Algorithm SHA256).Hash -ne $expectedHash) { throw 'Gitleaks download checksum mismatch.' }
            Move-Item -LiteralPath $download -Destination $archive
        } finally {
            if (Test-Path -LiteralPath $download) { Remove-Item -LiteralPath $download }
        }
    }
    if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $expectedHash) { throw 'Gitleaks archive checksum mismatch.' }
    Expand-Archive -LiteralPath $archive -DestinationPath $toolDirectory -Force
    & (Join-Path $toolDirectory 'gitleaks.exe') git $repositoryRoot "--log-opts=$RevisionRange" --redact=100 --no-banner --timeout=180
    if ($LASTEXITCODE -ne 0) { throw 'Credential scan failed. Review the redacted findings locally.' }
    $staged = git diff --cached --no-ext-diff --no-color
    if ($LASTEXITCODE -ne 0) { throw 'Unable to inspect staged changes.' }
    if ($staged) {
        $staged | & (Join-Path $toolDirectory 'gitleaks.exe') stdin --redact=100 --no-banner --timeout=180
        if ($LASTEXITCODE -ne 0) { throw 'Staged credential scan failed. Review the redacted findings locally.' }
    }
    Write-Output 'Repository privacy check passed.'
} finally {
    Pop-Location
}
