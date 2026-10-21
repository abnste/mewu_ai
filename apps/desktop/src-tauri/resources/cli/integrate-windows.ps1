# SPDX-License-Identifier: MPL-2.0
param(
    [ValidateSet('Install', 'Uninstall')][string]$Mode = 'Install',
    [string]$InstallDirectory,
    [switch]$LibraryOnly
)
$ErrorActionPreference = 'Stop'

# Keep PATH outside NSIS's bounded string registers. Read REG_EXPAND_SZ without
# expansion and preserve every unrelated entry and its original spelling.
function ConvertTo-MewuPathIdentity([string]$Value) {
    if ($null -eq $Value) { return $null }
    $candidate = $Value.Trim().Trim('"')
    if ($candidate.Length -eq 0) { return $null }
    $candidate = [Environment]::ExpandEnvironmentVariables($candidate)
    if ($candidate -notmatch '^(?:[A-Za-z]:[\\/]|\\\\[^\\]+\\[^\\]+)') { return $null }
    try {
        $candidate = [IO.Path]::GetFullPath($candidate)
        $rootLength = [IO.Path]::GetPathRoot($candidate).Length
        while ($candidate.Length -gt $rootLength -and ($candidate.EndsWith('\') -or $candidate.EndsWith('/'))) { $candidate = $candidate.Substring(0, $candidate.Length - 1) }
        return $candidate.ToUpperInvariant()
    } catch { return $null }
}
function Get-MewuPathPlan {
    param([string]$Action, [AllowEmptyString()][string]$CurrentPath, [bool]$PathExists, [string]$Directory, $Ownership)
    if ($Action -notin @('Install', 'Uninstall')) { throw 'CLI_PATH_INVALID_MODE' }
    $identity = ConvertTo-MewuPathIdentity $Directory
    if ($null -eq $identity -or $Directory.IndexOfAny([char[]]";`r`n`0") -ge 0) { throw 'CLI_PATH_INVALID_DIRECTORY' }
    $directory = [IO.Path]::GetFullPath($Directory)
    if ($directory.Length -gt [IO.Path]::GetPathRoot($directory).Length) { $directory = $directory.TrimEnd('\', '/') }
    $managed = $null -ne $Ownership -and $Ownership.schemaVersion -eq 1 -and $Ownership.managedBy -ceq 'dev.mewu.remake.cli.v1' -and $Ownership.installDirectory -is [string] -and $Ownership.addedEntry -is [string] -and $Ownership.createdPath -is [bool]
    if ($null -ne $Ownership -and (-not $managed -or ($Ownership.addedEntry.Length -gt 0 -and (ConvertTo-MewuPathIdentity $Ownership.addedEntry) -cne (ConvertTo-MewuPathIdentity $Ownership.installDirectory)))) { throw 'CLI_PATH_INVALID_OWNERSHIP' }
    $ownerMatches = $managed -and (ConvertTo-MewuPathIdentity $Ownership.installDirectory) -ceq $identity
    $parts = [Collections.Generic.List[string]]::new()
    foreach ($part in $CurrentPath.Split([char]';')) { $parts.Add($part) }
    $removed = $false
    # Only one exact token that this installer recorded as its own may be
    # removed. Normalized equality is only used for detecting existing PATHs.
    if ($managed -and ($Action -eq 'Install' -or $ownerMatches)) {
        for ($index = $parts.Count - 1; $index -ge 0; $index--) {
            if ($parts[$index] -ceq $Ownership.addedEntry -and $Ownership.addedEntry.Length -gt 0) {
                if ($Action -eq 'Uninstall' -or -not $ownerMatches) { $parts.RemoveAt($index); $removed = $true }
                break
            }
        }
    }
    $next = [string]::Join(';', $parts)
    if ($Action -eq 'Uninstall') {
        return [PSCustomObject]@{ path = $next; write = $removed -and $next -cne $CurrentPath; deletePath = $removed -and $next.Length -eq 0 -and $Ownership.createdPath; ownership = $null; clearOwnership = [bool]$ownerMatches }
    }
    $present = $false
    foreach ($part in $parts) { if ((ConvertTo-MewuPathIdentity $part) -ceq $identity) { $present = $true; break } }
    $stillOwned = $ownerMatches -and $parts.Contains($Ownership.addedEntry) -and $Ownership.addedEntry.Length -gt 0
    if (-not $present) {
        $next = if ($next.Length -eq 0) { $directory } else { $next + ';' + $directory }
        if ($next.Length -gt 32766) { throw 'CLI_PATH_TOO_LONG' }
    }
    $entry = if ($stillOwned) { $Ownership.addedEntry } elseif (-not $present) { $directory } else { '' }
    $created = if ($managed -and ($ownerMatches -or $removed)) { $Ownership.createdPath } else { -not $PathExists }
    $owner = [PSCustomObject]@{ schemaVersion = 1; managedBy = 'dev.mewu.remake.cli.v1'; installDirectory = $directory; addedEntry = $entry; createdPath = [bool]$created }
    return [PSCustomObject]@{ path = $next; write = $next -cne $CurrentPath -or -not $PathExists; deletePath = $false; ownership = $owner; clearOwnership = $false }
}

function Get-MewuUserPathSnapshot {
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $false)
    try {
        $exists = $null -ne $key -and $key.GetValueNames() -contains 'Path'
        $kind = if ($exists) { $key.GetValueKind('Path') } else { [Microsoft.Win32.RegistryValueKind]::ExpandString }
        if ($kind -notin @([Microsoft.Win32.RegistryValueKind]::String, [Microsoft.Win32.RegistryValueKind]::ExpandString)) { throw 'CLI_PATH_UNSUPPORTED_REGISTRY_TYPE' }
        $value = if ($exists) { [string]$key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames) } else { '' }
        return [PSCustomObject]@{ exists = $exists; kind = $kind; path = $value }
    } finally { if ($null -ne $key) { $key.Dispose() } }
}
function Send-MewuEnvironmentChange {
    # HWND_BROADCAST's timeout applies separately to every top-level window.
    # A hung unrelated app must not turn a successful registry write into an
    # installer failure. Keep the string alive on a background native call;
    # exiting this short-lived helper terminates any unfinished notification.
    try {
        if (-not ('MewuCliEnvironment' -as [type])) {
            Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Threading;
public static class MewuCliEnvironment {
    [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    private static extern IntPtr SendMessageTimeout(IntPtr h,uint m,IntPtr w,string l,uint f,uint t,out IntPtr r);
    public static bool Notify() {
        bool sent = false;
        var worker = new Thread(() => {
            try {
                IntPtr result;
                sent = SendMessageTimeout(new IntPtr(0xffff),0x1a,IntPtr.Zero,"Environment",2,200,out result) != IntPtr.Zero;
            } catch { }
        });
        worker.IsBackground = true;
        worker.Start();
        return worker.Join(2000) && sent;
    }
}
'@
        }
        return [MewuCliEnvironment]::Notify()
    } catch { return $false }
}
function Invoke-MewuWindowsIntegration([string]$Action, [string]$Directory) {
    if ($Action -eq 'Install' -and (-not [IO.File]::Exists([IO.Path]::Combine($Directory, 'mewu.exe')) -or -not [IO.File]::Exists([IO.Path]::Combine($Directory, 'MewuAI.exe')))) { throw 'CLI_PATH_PACKAGE_INCOMPLETE' }
    $sid = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    $mutex = [Threading.Mutex]::new($false, ('Local\MewuAI.CliPath.v1.' + $sid))
    $held = $false
    try {
        try { $held = $mutex.WaitOne(5000) } catch [Threading.AbandonedMutexException] { $held = $true }
        if (-not $held) { throw 'CLI_PATH_BUSY' }
        $stateKey = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software\MewuAI\CLIIntegration', $false)
        try { $state = if ($null -ne $stateKey) { $stateKey.GetValue('Ownership') } else { $null } }
        finally { if ($null -ne $stateKey) { $stateKey.Dispose() } }
        $ownership = $null
        if ($null -ne $state) {
            if ($state -isnot [string] -or $state.Length -gt 4096) { throw 'CLI_PATH_INVALID_OWNERSHIP' }
            $ownership = $state | ConvertFrom-Json
        }
        $done = $false
        for ($attempt = 0; $attempt -lt 4; $attempt++) {
            $before = Get-MewuUserPathSnapshot
            $plan = Get-MewuPathPlan -Action $Action -CurrentPath $before.path -PathExists $before.exists -Directory $Directory -Ownership $ownership
            $current = Get-MewuUserPathSnapshot
            if ($current.exists -ne $before.exists -or $current.kind -ne $before.kind -or $current.path -cne $before.path) { continue }
            if ($plan.write) {
                $environment = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Environment')
                try {
                    if ($plan.deletePath) { $environment.DeleteValue('Path', $false) }
                    else { $environment.SetValue('Path', $plan.path, $before.kind) }
                } finally { $environment.Dispose() }
            }
            if ($Action -eq 'Install') {
                $key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Software\MewuAI\CLIIntegration')
                try { $key.SetValue('Ownership', ($plan.ownership | ConvertTo-Json -Compress), [Microsoft.Win32.RegistryValueKind]::String) }
                finally { $key.Dispose() }
            } elseif ($plan.clearOwnership) {
                $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Software\MewuAI\CLIIntegration', $true)
                try { if ($null -ne $key) { $key.DeleteValue('Ownership', $false) } }
                finally { if ($null -ne $key) { $key.Dispose() } }
            }
            $verified = Get-MewuUserPathSnapshot
            $expectedExists = -not $plan.deletePath -and ($before.exists -or $plan.write)
            if ($verified.exists -ne $expectedExists -or ($expectedExists -and ($verified.path -cne $plan.path -or $verified.kind -ne $before.kind))) { throw 'CLI_PATH_VERIFY_FAILED' }
            $done = $true
            break
        }
        if (-not $done) { throw 'CLI_PATH_CHANGED_CONCURRENTLY' }
        $notificationSent = if ($plan.write) { Send-MewuEnvironmentChange } else { $true }
        return [PSCustomObject]@{ success = $true; changed = [bool]$plan.write; newShellRequired = [bool]$plan.write; notificationSent = [bool]$notificationSent }
    } finally { if ($held) { $mutex.ReleaseMutex() }; $mutex.Dispose() }
}

if ($LibraryOnly) { return }
try {
    Invoke-MewuWindowsIntegration $Mode $InstallDirectory | ConvertTo-Json -Compress
    exit 0
} catch {
    # Do not relay raw paths, registry contents or a PowerShell stack to NSIS.
    $code = if ($_.Exception.Message -cmatch '^CLI_PATH_[A-Z_]+$') { $_.Exception.Message } elseif ($_.Exception -is [UnauthorizedAccessException] -or $_.Exception -is [Security.SecurityException]) { 'CLI_PATH_ACCESS_DENIED' } else { 'CLI_PATH_INTEGRATION_FAILED' }
    [Console]::Error.WriteLine($code)
    exit 1
}
