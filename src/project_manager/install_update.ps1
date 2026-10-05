param([Parameter(Mandatory=$true)][string]$Plan)
$ErrorActionPreference='Stop'
# The parent may use PowerShell 7; its modules cannot load in Windows PowerShell 5.1.
$env:PSModulePath=Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\Modules'
$taskPlan=Get-Content -LiteralPath $Plan -Raw -Encoding UTF8 | ConvertFrom-Json
function FullPath([string]$value) { [IO.Path]::GetFullPath($value).TrimEnd('\') }
function SamePath([string]$left,[string]$right) { (FullPath $left).Equals((FullPath $right),[StringComparison]::OrdinalIgnoreCase) }
function RequireChild([string]$child,[string]$parent) {
    if (!(FullPath $child).StartsWith((FullPath $parent)+'\',[StringComparison]::OrdinalIgnoreCase)) { throw 'Path is outside its allowed directory.' }
}
function RequireNoLinks([string]$path) {
    $cursor=FullPath $path
    while ($cursor) {
        if (Test-Path -LiteralPath $cursor) {
            if ((Get-Item -LiteralPath $cursor -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Linked path rejected.' }
        }
        $parent=[IO.Path]::GetDirectoryName($cursor)
        if (!$parent -or $parent -eq $cursor) { break }
        $cursor=$parent
    }
}
function WriteResult([string]$state,[string]$message) {
    $record=@{ state=$state; message=$message; version=$taskPlan.version; backup=$taskPlan.backup }
    [IO.File]::WriteAllText($taskPlan.result,($record|ConvertTo-Json),[Text.UTF8Encoding]::new($false))
}
function WritePhase([string]$phase) {
    $record=@{ id=$taskPlan.id; phase=$phase; install_dir=$installationPath; backup=$backupPath }
    $temporary=$statusPath+'.tmp'
    [IO.File]::WriteAllText($temporary,($record|ConvertTo-Json),[Text.UTF8Encoding]::new($false))
    Move-Item -LiteralPath $temporary -Destination $statusPath -Force
}
function StartPrevious {
    try { Start-Process -FilePath (Join-Path $installationPath 'CodexManager.exe') -ArgumentList @('--state-dir',('"'+$taskPlan.state_dir+'"'),'--no-update-check') -WorkingDirectory $installationPath -WindowStyle Normal } catch {}
}
function RemoveEmptyFolders([string]$folder) {
    foreach ($child in @(Get-ChildItem -LiteralPath $folder -Directory -Force)) {
        if (!($child.Attributes -band [IO.FileAttributes]::ReparsePoint)) { RemoveEmptyFolders $child.FullName }
    }
    if (@(Get-ChildItem -LiteralPath $folder -Force).Count -eq 0) { Remove-Item -LiteralPath $folder }
}
$installationPath=FullPath $taskPlan.install_dir
$stagedPath=FullPath $taskPlan.staged
$backupPath=FullPath $taskPlan.backup
$parentDirectory=[IO.Path]::GetDirectoryName($installationPath)
$swapped=$false;$newProcess=$null;$healthy=$false;$pathsValidated=$false
try {
    if (!$parentDirectory -or (SamePath $installationPath ([IO.Path]::GetPathRoot($installationPath)))) { throw 'Drive root cannot be replaced.' }
    RequireChild $stagedPath $parentDirectory;RequireChild $backupPath $parentDirectory
    if (!(SamePath ([IO.Path]::GetDirectoryName($backupPath)) $parentDirectory) -or
        !([IO.Path]::GetFileName($backupPath)).StartsWith('.codex-manager-previous-')) { throw 'Backup path rejected.' }
    $stageDirectory=[IO.Path]::GetDirectoryName($stagedPath)
    if (!(SamePath ([IO.Path]::GetDirectoryName($stageDirectory)) $parentDirectory) -or
        !([IO.Path]::GetFileName($stageDirectory)).StartsWith('.codex-manager-update-') -or
        [IO.Path]::GetFileName($stagedPath) -ne 'CodexManager') { throw 'Staged path rejected.' }
    $updateState=Join-Path $taskPlan.state_dir 'updates'
    RequireChild $taskPlan.health $updateState;RequireChild $taskPlan.result $updateState
    $statusPath=Join-Path $updateState ($taskPlan.id+'.status.json');RequireChild $statusPath $updateState
    if ((SamePath $taskPlan.state_dir $installationPath) -or (FullPath $taskPlan.state_dir).StartsWith($installationPath+'\',[StringComparison]::OrdinalIgnoreCase)) { throw 'Settings must remain outside application.' }
    RequireNoLinks $installationPath;RequireNoLinks $stagedPath;RequireNoLinks $backupPath;RequireNoLinks $updateState
    $pathsValidated=$true
    Set-Location -LiteralPath $updateState
    $previousStatus=$null
    if (Test-Path -LiteralPath $statusPath) { $previousStatus=Get-Content -LiteralPath $statusPath -Raw -Encoding UTF8 | ConvertFrom-Json }
    if ($previousStatus.phase -in @('completed','recovered','failed')) { throw 'This update has already finished.' }
    if (Test-Path -LiteralPath $backupPath) {
        if (!$previousStatus -or $previousStatus.id -ne $taskPlan.id -or !(SamePath $previousStatus.install_dir $installationPath) -or !(SamePath $previousStatus.backup $backupPath)) { throw 'Unrecognized previous installation.' }
        foreach ($other in @(Get-Process -Name CodexManager -ErrorAction SilentlyContinue)) {
            if ($other.Path -and (SamePath $other.Path (Join-Path $installationPath 'CodexManager.exe'))) { throw 'Close the application before recovering.' }
        }
        if (Test-Path -LiteralPath $installationPath) {
            $manifestPath=Join-Path $installationPath 'install-manifest.json';RequireNoLinks $manifestPath
            if (!$taskPlan.manifest_sha256 -or !(Test-Path -LiteralPath $manifestPath) -or (Get-FileHash -LiteralPath $manifestPath -Algorithm SHA256).Hash -ne $taskPlan.manifest_sha256) { throw 'Changed installation retained; manual recovery needed.' }
            $failedPath=$installationPath+'.failed-'+$taskPlan.id;RequireChild $failedPath $parentDirectory;RequireNoLinks $failedPath
            if (Test-Path -LiteralPath $failedPath) { throw 'Recovery destination already exists.' }
            Move-Item -LiteralPath $installationPath -Destination $failedPath
        }
        Move-Item -LiteralPath $backupPath -Destination $installationPath
        WritePhase 'recovered';WriteResult 'recovered' 'Interrupted update restored to previous program.';StartPrevious;exit 0
    }
    $newManifest=Get-Content -LiteralPath (Join-Path $stagedPath 'install-manifest.json') -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($newManifest.version -ne $taskPlan.version) { throw 'Version mismatch.' }
    foreach ($file in $newManifest.files.PSObject.Properties) {
        $path=Join-Path $stagedPath $file.Name;RequireChild $path $stagedPath;RequireNoLinks $path
        if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ne $file.Value) { throw 'Staged hash mismatch.' }
    }
    if (!(Test-Path -LiteralPath (Join-Path $installationPath 'CodexManager.exe')) -or !(Test-Path -LiteralPath (Join-Path $stagedPath 'CodexManager.exe'))) { throw 'Executable missing.' }
    $parentProcess=Get-Process -Id $taskPlan.parent_pid -ErrorAction SilentlyContinue
    if ($parentProcess) { $parentProcess|Wait-Process -Timeout 60 }
    foreach ($other in @(Get-Process -Name CodexManager -ErrorAction SilentlyContinue)) {
        if ($other.Path -and (SamePath $other.Path (Join-Path $installationPath 'CodexManager.exe'))) { throw 'Another app instance is running.' }
    }
    WritePhase 'replacing'
    Move-Item -LiteralPath $installationPath -Destination $backupPath
    try { Move-Item -LiteralPath $stagedPath -Destination $installationPath }
    catch { Move-Item -LiteralPath $backupPath -Destination $installationPath;StartPrevious;throw }
    $swapped=$true
    WritePhase 'starting'
    $arguments=@('--state-dir',('"'+$taskPlan.state_dir+'"'),'--update-health',('"'+$taskPlan.health+'"'),'--no-update-check')
    $newProcess=Start-Process -FilePath (Join-Path $installationPath 'CodexManager.exe') -ArgumentList $arguments -WorkingDirectory $installationPath -WindowStyle Normal -PassThru
    $deadline=(Get-Date).AddSeconds(60);$healthy=$false
    while ((Get-Date) -lt $deadline) {
        if (Test-Path -LiteralPath $taskPlan.health) {
            $health=Get-Content -LiteralPath $taskPlan.health -Raw -Encoding UTF8 | ConvertFrom-Json
            if ($health.version -eq $taskPlan.version -and (SamePath $health.install_dir $installationPath) -and $health.pid -eq $newProcess.Id) { $healthy=$true;break }
        }
        $newProcess.Refresh();if ($newProcess.HasExited) { break };Start-Sleep -Milliseconds 250
    }
    if (!$healthy) { throw 'New application did not report healthy startup.' }
    WritePhase 'completed'
    # Remove only hash-verified old program files after replacement starts.
    # Unknown or changed user files remain in the previous directory.
    $cleanupMessage='Update installed and startup verified.'
    try {
      $oldManifestPath=Join-Path $backupPath 'install-manifest.json'
      if (Test-Path -LiteralPath $oldManifestPath) {
        $oldHash=(Get-FileHash -LiteralPath $oldManifestPath -Algorithm SHA256).Hash
        $oldManifest=Get-Content -LiteralPath $oldManifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
        foreach ($file in $oldManifest.files.PSObject.Properties) {
            try {
                $path=Join-Path $backupPath $file.Name;RequireChild $path $backupPath;RequireNoLinks $path
                if ((Test-Path -LiteralPath $path -PathType Leaf) -and (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -eq $file.Value) { Remove-Item -LiteralPath $path }
            } catch { } # Retain changed, unreadable, or linked items.
        }
        if ((Get-FileHash -LiteralPath $oldManifestPath -Algorithm SHA256).Hash -eq $oldHash) { Remove-Item -LiteralPath $oldManifestPath }
        RemoveEmptyFolders $backupPath
      }
      if ((Test-Path -LiteralPath $stageDirectory) -and @(Get-ChildItem -LiteralPath $stageDirectory -Force).Count -eq 0) { Remove-Item -LiteralPath $stageDirectory }
    } catch { $cleanupMessage='Update installed; some previous files were retained: '+$_.Exception.Message }
    WriteResult 'completed' $cleanupMessage
} catch {
    $message=$_.Exception.Message
    if ($swapped -and !$healthy -and (Test-Path -LiteralPath $backupPath)) {
        if ($newProcess) { try { if (!$newProcess.HasExited) { $newProcess.Kill();$newProcess.WaitForExit() } } catch {} }
        $failedPath=$installationPath+'.failed-'+$taskPlan.id
        RequireChild $failedPath $parentDirectory;RequireNoLinks $failedPath
        if (Test-Path -LiteralPath $installationPath) { Move-Item -LiteralPath $installationPath -Destination $failedPath }
        Move-Item -LiteralPath $backupPath -Destination $installationPath
        WritePhase 'failed';StartPrevious
    }
    if ($pathsValidated) { try { WriteResult 'failed' $message } catch {} }
    exit 1
}
