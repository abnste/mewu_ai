# SPDX-License-Identifier: MPL-2.0
param([string]$Helper, [string]$Directory)
$ErrorActionPreference='Stop'
. $Helper -LibraryOnly
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class MewuTestRegistry {
    [DllImport("advapi32.dll")]
    public static extern int RegOverridePredefKey(IntPtr key, IntPtr replacement);
}
'@
function Assert-MewuFixture($Value) { if (-not $Value) { throw 'Registry integration assertion failed' } }
$fixtureName='Software\MewuAI\CLIIntegrationTests\'+[Guid]::NewGuid().ToString('N')
$original=[Microsoft.Win32.Registry]::CurrentUser
$before=Get-MewuUserPathSnapshot
$fixture=$original.CreateSubKey($fixtureName)
$hkcu=[IntPtr]::new(-2147483647)
$overridden=$false
try {
    Assert-MewuFixture ([MewuTestRegistry]::RegOverridePredefKey($hkcu,$fixture.Handle.DangerousGetHandle()) -eq 0)
    $overridden=$true
    $initial=(@(1..150 | ForEach-Object { '%USERPROFILE%\Test tools '+$_ }) -join ';')+';'
    $envKey=[Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Environment')
    try { $envKey.SetValue('Path',$initial,[Microsoft.Win32.RegistryValueKind]::ExpandString) } finally { $envKey.Dispose() }
    # Model an unavailable desktop notification; actual registry integration
    # must still succeed and preserve every unrelated unexpanded token.
    function Send-MewuEnvironmentChange { return $false }
    $first=Invoke-MewuWindowsIntegration Install $Directory
    $installed=Get-MewuUserPathSnapshot
    Assert-MewuFixture ($first.success -and $first.changed -and -not $first.notificationSent)
    Assert-MewuFixture ($installed.path -ceq ($initial+';'+[IO.Path]::GetFullPath($Directory)))
    Assert-MewuFixture ($installed.kind -eq [Microsoft.Win32.RegistryValueKind]::ExpandString)
    $again=Invoke-MewuWindowsIntegration Install $Directory
    Assert-MewuFixture ($again.success -and -not $again.changed)
    $removed=Invoke-MewuWindowsIntegration Uninstall $Directory
    Assert-MewuFixture ($removed.success -and (Get-MewuUserPathSnapshot).path -ceq $initial)
    $envKey=[Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment',$true)
    try { $envKey.DeleteValue('Path') } finally { $envKey.Dispose() }
    $fresh=Invoke-MewuWindowsIntegration Install $Directory
    Assert-MewuFixture ($fresh.success -and (Get-MewuUserPathSnapshot).exists)
    $removed=Invoke-MewuWindowsIntegration Uninstall $Directory
    Assert-MewuFixture ($removed.success -and -not (Get-MewuUserPathSnapshot).exists)
    [PSCustomObject]@{success=$true;bits=[IntPtr]::Size*8;notificationFailurePreservesInstall=$true;repeatAndUninstall=$true;missingPathRestored=$true} | ConvertTo-Json -Compress
} finally {
    if ($overridden) { Assert-MewuFixture ([MewuTestRegistry]::RegOverridePredefKey($hkcu,[IntPtr]::Zero) -eq 0) }
    $fixture.Dispose()
    $after=Get-MewuUserPathSnapshot
    Assert-MewuFixture ($after.exists -eq $before.exists -and $after.path -ceq $before.path -and $after.kind -eq $before.kind)
    $original.DeleteSubKeyTree($fixtureName)
}
