# SPDX-License-Identifier: MPL-2.0
# Daily UI-only update. This is not a GUI or release acceptance test.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$Candidate,
    [Parameter(Mandatory)][ValidatePattern('^[a-fA-F0-9]{64}$')][string]$ExpectedSha256,
    [Parameter(Mandatory)][ValidatePattern('^[a-f0-9]{40}$')][string]$SourceCommit,
    [Parameter(Mandatory)][ValidatePattern('^[a-f0-9]{40}$')][string]$BaselineCommit,
    [Parameter(Mandatory)][switch]$UiOnly,
    [string]$WindowLifecycleReview,
    [string]$SourceRoot
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (-not $IsWindows) { throw 'This updater is Windows-only' }
if (-not $UiOnly) { throw 'UI-only scope must be explicit' }
$taskRepo = [IO.Path]::GetFullPath($(if ($SourceRoot) { $SourceRoot } else { Join-Path $PSScriptRoot '../..' }))
$taskInstall = Join-Path $env:LOCALAPPDATA 'MewuAI/MewuAI.exe'
$taskPrivate = Join-Path $taskRepo '.private/dev-check'
$taskRun = 'daily-' + [Guid]::NewGuid().ToString('N')
$taskBackup = Join-Path $env:LOCALAPPDATA ('MewuAI-remake-backups/' + $taskRun)
$taskReceiptPath = Join-Path $taskPrivate ($taskRun + '.json')
$taskStage = 'preflight'
$taskWatch = [Diagnostics.Stopwatch]::StartNew()
$taskReceipt = [ordered]@{schema=1;kind='windows-update';passed=$false;sourceCommit=$SourceCommit;baselineCommit=$BaselineCommit;candidateSha256=$ExpectedSha256.ToLowerInvariant();startedAtUtc=[DateTime]::UtcNow.ToString('O');forcedTermination=$false;nativeProductGuiVerified=$false;published=$false}
$taskCandidatePin = $null
$taskLock = $null

function Git([string[]]$ArgsList) {
    $text = & (Get-Command git.exe -CommandType Application -ErrorAction Stop | Select-Object -First 1).Source --no-optional-locks -C $taskRepo @ArgsList
    if ($LASTEXITCODE -ne 0) { throw 'Git source inspection failed' }
    return [string]::Join("`n", @($text))
}
function Inside([string]$Base, [string]$Target) {
    $a=[IO.Path]::GetFullPath($Base).TrimEnd('\'); $b=[IO.Path]::GetFullPath($Target)
    return $b.Equals($a,[StringComparison]::OrdinalIgnoreCase) -or $b.StartsWith($a+'\',[StringComparison]::OrdinalIgnoreCase)
}
function Ordinary([string]$Target) {
    $full=[IO.Path]::GetFullPath($Target); $cursor=$full
    while ($cursor) {
        if (Test-Path -LiteralPath $cursor) {
            $item=Get-Item -LiteralPath $cursor -Force
            if (($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'Reparse points are not allowed' }
        }
        $parent=[IO.Directory]::GetParent($cursor)
        if ($null -eq $parent) { break }; $cursor=$parent.FullName
    }
    return Get-Item -LiteralPath $full -Force
}
function OwnedProcesses {
    $found=@()
    foreach ($item in @(Get-CimInstance Win32_Process -Filter "Name = 'MewuAI.exe'")) {
        if (-not $item.ExecutablePath) { throw 'Cannot identify a Mewu process; refusing shutdown' }
        if ([IO.Path]::GetFullPath($item.ExecutablePath).Equals([IO.Path]::GetFullPath($taskInstall),[StringComparison]::OrdinalIgnoreCase)) { $found += $item }
    }
    return $found
}
function Cold {
    if (@(OwnedProcesses).Count) { throw 'Installed application or worker is still running' }
}
function Scope {
    $null=Ordinary $taskRepo
    if (-not (Git @('rev-parse','--show-toplevel')).Trim().Equals($taskRepo.Replace('\','/'),[StringComparison]::OrdinalIgnoreCase)) { throw 'Source root must be the Git checkout root' }
    if ((Git @('rev-parse','HEAD')).Trim() -cne $SourceCommit -or (Git @('status','--porcelain')).Length) { throw 'Candidate source must be current and clean' }
    $null=Git @('cat-file','-e',($BaselineCommit+'^{commit}'))
    $null=Git @('merge-base','--is-ancestor',$BaselineCommit,$SourceCommit)
    $paths=@((Git @('diff','--name-only','-z',$BaselineCommit,$SourceCommit)).Split([char]0,[StringSplitOptions]::RemoveEmptyEntries))
    foreach ($name in $paths) {
        if ($name -eq 'apps/desktop/src-tauri/src/main.rs' -and $WindowLifecycleReview) {
            $reviewPath=[IO.Path]::GetFullPath($WindowLifecycleReview)
            if (-not (Inside $taskPrivate $reviewPath) -or (Ordinary $reviewPath).PSIsContainer) { throw 'Lifecycle review must be a private local JSON file' }
            $review=Get-Content -LiteralPath $reviewPath -Raw|ConvertFrom-Json
            if ($review.kind -cne 'window-lifecycle-only' -or $review.sourceCommit -cne $SourceCommit -or $review.baselineCommit -cne $BaselineCommit -or $review.path -cne $name -or $review.beforeBlob -cne (Git @('rev-parse',($BaselineCommit+':'+$name))).Trim() -or $review.afterBlob -cne (Git @('rev-parse',($SourceCommit+':'+$name))).Trim()) { throw 'Reviewed window-lifecycle change differs' }
            continue
        }
        if ($name -eq 'Cargo.toml') {
            $python=Join-Path $env:USERPROFILE '.cache/codex-runtimes/codex-primary-runtime/dependencies/python/python.exe'
            if (-not (Test-Path -LiteralPath $python)) { throw 'Profile-only Cargo inspection requires the configured Python runtime' }
            $payload=@{before=(Git @('show',($BaselineCommit+':Cargo.toml')));after=(Git @('show',($SourceCommit+':Cargo.toml')))} | ConvertTo-Json -Compress
            $payload | & $python -c 'import json,sys,tomllib; p=json.load(sys.stdin); a=tomllib.loads(p["before"]); b=tomllib.loads(p["after"]); a.pop("profile",None); b.pop("profile",None); sys.exit(0 if a==b else 1)'
            if ($LASTEXITCODE -ne 0) { throw 'Cargo dependencies/workspace changed; use dedicated checks' }
            continue
        }
        if ($name -match '(?i)(?:storage|schema|migration|keychain|credential|app[-_]update|data[-_]directory|data[-_]cleanup|/types\.ts$|/model\.ts$|/bridge\.ts$)') { throw 'Data, credentials or updater changed; use dedicated checks' }
        if ($name -match '^apps/desktop/src/' -or $name -eq 'apps/desktop/index.html' -or $name -eq 'apps/desktop/src-tauri/src/frozen.rs' -or $name -match '^scripts/(?:dev-check|linux-tiff-no-jbig)/' -or $name -match '^docs/[^\r\n]+\.(?:md|svg|png)$' -or $name -match '^(?:README(?:\.[A-Za-z-]+)?\.md|LICENSE)$') { continue }
        throw 'Changed file is outside the UI-only update scope; use dedicated checks'
    }
    return $paths
}
function BackupTree([string]$Source, [string]$Destination) {
    if (-not (Inside $taskBackup $Destination) -or (Test-Path -LiteralPath $Destination) -or (Inside $Source $Destination) -or (Inside $Destination $Source)) { throw 'Backup containment failed' }
    $rootItem=Ordinary $Source
    if (-not $rootItem.PSIsContainer) { throw 'Backup root must be a directory' }
    $queue=[Collections.Generic.Queue[string]]::new(); $queue.Enqueue($Source)
    $files=[Collections.Generic.List[object]]::new(); $directories=[Collections.Generic.List[string]]::new()
    $null=New-Item -ItemType Directory -Path $Destination
    while ($queue.Count) {
        $current=$queue.Dequeue()
        foreach ($item in Get-ChildItem -LiteralPath $current -Force) {
            $null=Ordinary $item.FullName
            $relative=[IO.Path]::GetRelativePath($Source,$item.FullName)
            $target=Join-Path $Destination $relative
            if (-not (Inside $Destination $target)) { throw 'Backup member escapes destination' }
            if ($item.PSIsContainer) { $null=New-Item -ItemType Directory -Path $target; $directories.Add($relative); $queue.Enqueue($item.FullName) }
            else {
                $hash=(Get-FileHash -LiteralPath $item.FullName -Algorithm SHA256).Hash
                Copy-Item -LiteralPath $item.FullName -Destination $target
                if ((Get-Item -LiteralPath $target).Length -ne $item.Length -or (Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash -cne $hash) { throw 'Cold backup copy differs' }
                $files.Add([ordered]@{name=$relative;bytes=$item.Length;sha256=$hash.ToLowerInvariant()})
            }
        }
    }
    return [ordered]@{files=@($files);directories=@($directories)}
}
function WriteReceipt {
    $taskReceipt.finishedAtUtc=[DateTime]::UtcNow.ToString('O'); $taskReceipt.elapsedMs=$taskWatch.ElapsedMilliseconds
    $bytes=[Text.UTF8Encoding]::new($false).GetBytes(($taskReceipt|ConvertTo-Json -Depth 8))
    $stream=[IO.File]::Open($taskReceiptPath,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
    try { $stream.Write($bytes,0,$bytes.Length); $stream.Flush($true) } finally { $stream.Dispose() }
}
try {
    $taskReceipt.changedFiles=@(Scope)
    $Candidate=[IO.Path]::GetFullPath((Resolve-Path -LiteralPath $Candidate).Path)
    if (-not (Inside (Join-Path $taskRepo 'out') $Candidate) -or (Ordinary $Candidate).PSIsContainer -or [IO.Path]::GetExtension($Candidate) -ine '.exe') { throw 'Candidate must be an ordinary executable under this repository/out' }
    $taskCandidatePin=[IO.File]::Open($Candidate,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
    if ((Get-FileHash -LiteralPath $Candidate -Algorithm SHA256).Hash -ine $ExpectedSha256) { throw 'Candidate SHA256 differs' }
    if ((Ordinary $taskInstall).PSIsContainer) { throw 'Installed executable is invalid' }
    $taskReceipt.oldSha256=(Get-FileHash -LiteralPath $taskInstall -Algorithm SHA256).Hash.ToLowerInvariant()
    $null=Ordinary (Split-Path -Parent $taskPrivate)
    $null=New-Item -ItemType Directory -Path $taskPrivate -Force
    $null=Ordinary $taskPrivate
    $taskLock=[IO.File]::Open((Join-Path $taskPrivate 'windows-update.lock'),[IO.FileMode]::OpenOrCreate,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None)
    $processes=@(OwnedProcesses)
    if ($processes.Where({$_.CommandLine -match '(?:^|\s)--recording-worker(?:\s|$)'}).Count) { throw 'Recording worker is active; stop recording normally before updating' }
    if ($processes.Count -gt 1) { throw 'Multiple installed hosts exist; refusing shutdown' }
    $taskReceipt.oldAlreadyExited=($processes.Count -eq 0)
    if ($processes.Count -eq 1) {
        $taskStage='normal-quit'
        $old=Get-Process -Id $processes[0].ProcessId
        $oldStart=$old.StartTime.ToUniversalTime().ToString('O')
        if (-not $old.Path.Equals($taskInstall,[StringComparison]::OrdinalIgnoreCase)) { throw 'Old process identity changed' }
        $taskReceipt.oldPid=$old.Id; $taskReceipt.oldStartedAtUtc=$oldStart
        if ((Get-FileHash -LiteralPath $taskInstall).Hash.ToLowerInvariant() -cne $taskReceipt.oldSha256) { throw 'Installed executable changed' }
        $quit=Start-Process -FilePath $taskInstall -ArgumentList '--quit' -WindowStyle Hidden -PassThru
        if (-not $quit.WaitForExit(15000) -or $quit.ExitCode -ne 0 -or -not $old.WaitForExit(60000)) { throw 'Normal shutdown timed out; no process was forcibly terminated' }
        $taskReceipt.oldCooperativeQuit=$true
    }
    Cold
    $taskStage='storage-query'
    $infoPath=Join-Path $taskPrivate ($taskRun+'-storage.json')
    $probe=Start-Process -FilePath $taskInstall -ArgumentList @('--storage-info',('"'+$infoPath+'"')) -WindowStyle Hidden -PassThru
    if (-not $probe.WaitForExit(15000) -or $probe.ExitCode -ne 0) { throw 'Read-only storage query failed' }
    $info=Get-Content -LiteralPath $infoPath -Raw | ConvertFrom-Json
    if ($info.phase -cne 'idle' -or $null -ne $info.pendingId) { throw 'Storage is not idle; use dedicated checks' }
    Cold
    $sources=@([IO.Path]::GetFullPath($info.root),[IO.Path]::GetFullPath($info.bootstrap),(Split-Path -Parent $taskInstall))
    foreach ($item in $sources) {
        $null=Ordinary $item
        if ((Inside $item $taskBackup) -or (Inside $taskBackup $item)) { throw 'Backup overlaps live data' }
    }
    $taskStage='cold-backup'
    $null=Ordinary (Split-Path -Parent $taskBackup)
    if (Test-Path -LiteralPath $taskBackup) { throw 'Backup already exists' }
    $null=New-Item -ItemType Directory -Path $taskBackup
    $inventories=[ordered]@{}
    for ($i=0;$i -lt $sources.Count;$i++) { Cold; $label=@('data','bootstrap','program')[$i]; $inventories[$label]=BackupTree $sources[$i] (Join-Path $taskBackup $label) }
    [IO.File]::WriteAllText((Join-Path $taskBackup 'inventory.json'),($inventories|ConvertTo-Json -Depth 8),[Text.UTF8Encoding]::new($false))
    $taskReceipt.backup=$taskBackup; $taskReceipt.backupCopyVerified=$true; $taskReceipt.storageGeneration=$info.generation
    $taskStage='replace'
    $null=Scope; Cold
    if ((Get-FileHash -LiteralPath $taskInstall).Hash.ToLowerInvariant() -cne $taskReceipt.oldSha256) { throw 'Installed binary changed since backup' }
    Copy-Item -LiteralPath $Candidate -Destination $taskInstall -Force
    if ((Get-FileHash -LiteralPath $taskInstall).Hash -ine $ExpectedSha256) { throw 'Installed replacement SHA256 differs; cold backup retained' }
    $taskStage='launch'
    $new=Start-Process -FilePath $taskInstall -WorkingDirectory (Split-Path -Parent $taskInstall) -WindowStyle Hidden -PassThru
    if ($new.WaitForExit(3000)) { throw 'Application exited immediately after launch; cold backup retained' }
    if (-not $new.Path.Equals($taskInstall,[StringComparison]::OrdinalIgnoreCase)) { throw 'Launched process identity differs' }
    $taskReceipt.pid=$new.Id; $taskReceipt.startedAtUtcProcess=$new.StartTime.ToUniversalTime().ToString('O'); $taskReceipt.installedSha256=$ExpectedSha256.ToLowerInvariant()
    $taskReceipt.passed=$true; $taskReceipt.stage='installed-and-launched'; $taskReceipt.guiAcceptance='pending-user'; WriteReceipt
    Write-Output ($taskReceipt|ConvertTo-Json -Depth 8 -Compress)
} catch {
    $taskReceipt.stage=$taskStage
    # Do not copy arbitrary exception text, paths, credentials or business state into reports.
    $taskReceipt.failure='Update stopped at '+$taskStage+'; backup and previous receipts retained'
    if (Test-Path -LiteralPath $taskPrivate) { try { WriteReceipt } catch { } }
    throw
} finally {
    if ($null -ne $taskCandidatePin) { $taskCandidatePin.Dispose() }
    if ($null -ne $taskLock) { $taskLock.Dispose() }
}
