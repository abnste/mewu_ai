param(
 [Parameter(Mandatory=$true)][int]$RootProcessId,
 [Parameter(Mandatory=$true)][ValidatePattern('^[a-f0-9]{40}$')][string]$SourceCommit,
 [Parameter(Mandatory=$true)][ValidatePattern('^[A-Fa-f0-9]{64}$')][string]$ExecutableSha256,
 [Parameter(Mandatory=$true)][string]$Output,
 [Parameter(Mandatory=$true)][string]$Scenario
)
$ErrorActionPreference='Stop'
$taskRepository=[IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$taskPrivate=[IO.Path]::GetFullPath((Join-Path $taskRepository '.private'))
$taskOutput=[IO.Path]::GetFullPath($Output)
if(-not $taskOutput.StartsWith($taskPrivate+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)-or(Test-Path -LiteralPath $taskOutput)){throw 'Output must be a new private evidence file'}
$taskExe=Join-Path $env:LOCALAPPDATA 'MewuAI/MewuAI.exe'
$taskRoot=Get-Process -Id $RootProcessId -ErrorAction Stop
if($taskRoot.Path-cne[IO.Path]::GetFullPath($taskExe)-or(Get-FileHash -LiteralPath $taskExe).Hash-cne$ExecutableSha256.ToUpperInvariant()){throw 'Root executable differs'}
$taskStart=$taskRoot.StartTime.ToUniversalTime().ToString('O')
$taskProcesses=@(Get-CimInstance Win32_Process)
$taskOwned=@($RootProcessId)
$taskQueue=[Collections.Generic.Queue[int]]::new();$taskQueue.Enqueue($RootProcessId)
while($taskQueue.Count){
 $taskParent=$taskQueue.Dequeue()
 $taskParentMeta=@($taskProcesses|Where-Object{$_.ProcessId-eq$taskParent})
 if($taskParentMeta.Count-ne1){throw 'Parent metadata missing'}
 foreach($taskChild in $taskProcesses|Where-Object{$_.ParentProcessId-eq$taskParent-and$_.Name-ceq'msedgewebview2.exe'}){
  if($taskChild.CreationDate-lt$taskParentMeta[0].CreationDate-or$taskOwned-contains[int]$taskChild.ProcessId){throw 'Process ownership ambiguous'}
  $taskOwned+=[int]$taskChild.ProcessId;$taskQueue.Enqueue([int]$taskChild.ProcessId)
 }
}
$taskRows=@();$taskWorking=[long]0;$taskPrivateBytes=[long]0
foreach($taskId in $taskOwned){
 $taskProcess=Get-Process -Id $taskId -ErrorAction Stop
 $taskMeta=@($taskProcesses|Where-Object{$_.ProcessId-eq$taskId})[0]
 $taskTicks=$taskProcess.StartTime.ToUniversalTime().Ticks
 if(($taskTicks-($taskTicks%10))-ne$taskMeta.CreationDate.ToUniversalTime().Ticks){throw 'Process lifetime changed'}
 $taskWorking+=$taskProcess.WorkingSet64;$taskPrivateBytes+=$taskProcess.PrivateMemorySize64
 $taskRows+=[ordered]@{pid=$taskId;name=$taskProcess.ProcessName;workingSetBytes=$taskProcess.WorkingSet64;privateBytes=$taskProcess.PrivateMemorySize64}
}
if((Get-Process -Id $RootProcessId).StartTime.ToUniversalTime().ToString('O')-cne$taskStart){throw 'Root lifetime changed'}
$taskResult=[ordered]@{sourceCommit=$SourceCommit;executableSha256=$ExecutableSha256.ToUpperInvariant();sampleUtc=[DateTime]::UtcNow.ToString('O');scenario=$Scenario;rootPid=$RootProcessId;rootStartedAtUtc=$taskStart;processes=$taskRows;treeWorkingSetBytes=$taskWorking;treePrivateBytes=$taskPrivateBytes;sharedWorkingPagesCanBeDoubleCounted=$true;nativeGuiVerified=$false}
[IO.Directory]::CreateDirectory((Split-Path -Parent $taskOutput))|Out-Null
[IO.File]::WriteAllText($taskOutput,($taskResult|ConvertTo-Json -Depth 5)+"`n",[Text.UTF8Encoding]::new($false))
$taskResult|ConvertTo-Json -Depth 5
