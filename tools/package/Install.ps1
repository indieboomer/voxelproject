param(
    [string]$Destination = (Join-Path $env:LOCALAPPDATA 'Programs\Voxel Project'),
    [switch]$NoShortcuts
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$source = [IO.Path]::GetFullPath($PSScriptRoot).TrimEnd('\')
$destinationPath = [IO.Path]::GetFullPath($Destination).TrimEnd('\')
if ($source -eq $destinationPath -or $destinationPath.StartsWith($source + '\', [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Install outside the extracted package folder, or use Play.bat for portable play.'
}
if (Get-Process voxelproject -ErrorAction SilentlyContinue) { throw 'Close the game before installing or updating.' }
# Avoid updating a server DLL or model still in use by this installation.
$activeServers = Get-Process llama-server -ErrorAction SilentlyContinue
foreach ($server in $activeServers) {
    try { $serverPath = $server.Path } catch { $serverPath = $null }
    if ($serverPath -and $serverPath.StartsWith($destinationPath + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw 'The installed AI server is still running. Close llama-server in Task Manager before updating.'
    }
}
$manifest = Get-Content -LiteralPath (Join-Path $source 'package-manifest.json') -Raw | ConvertFrom-Json
# Validate every path before writing, including manifests from a downloaded package.
foreach ($file in $manifest.files) {
    foreach ($base in @($source, $destinationPath)) {
        $resolved = [IO.Path]::GetFullPath((Join-Path $base $file.path))
        if (![string]::IsNullOrEmpty([IO.Path]::GetPathRoot($file.path)) -or
            !$resolved.StartsWith($base + '\', [StringComparison]::OrdinalIgnoreCase)) {
            throw "Invalid package path: $($file.path)"
        }
    }
}
Write-Host 'Checking package integrity (the AI model is several GB)...'
foreach ($file in $manifest.files) {
    $path = Join-Path $source $file.path
    if (!(Test-Path -LiteralPath $path -PathType Leaf) -or
        (Get-Item -LiteralPath $path).Length -ne $file.bytes -or
        (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ne $file.sha256) {
        throw "Missing or damaged package file: $($file.path). Extract the ZIP again."
    }
}
New-Item -ItemType Directory -Path $destinationPath -Force | Out-Null
Write-Host "Installing to $destinationPath ..."
foreach ($file in $manifest.files) {
    # Personal data is never part of a release; keep settings on an upgrade.
    if ($file.path -like 'saves/*' -or $file.path -like 'saves\*') { continue }
    $target = Join-Path $destinationPath $file.path
    if ($file.path -eq 'settings.json' -and (Test-Path -LiteralPath $target)) { continue }
    New-Item -ItemType Directory -Path (Split-Path -Parent $target) -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $source $file.path) -Destination $target -Force
}
Copy-Item -LiteralPath (Join-Path $source 'package-manifest.json') -Destination $destinationPath -Force
if (!$NoShortcuts) {
    $shell = New-Object -ComObject WScript.Shell
    $shortcutDirs = @([Environment]::GetFolderPath('Desktop'),
        (Join-Path ([Environment]::GetFolderPath('Programs')) 'Voxel Project'))
    foreach ($directory in $shortcutDirs) {
        New-Item -ItemType Directory -Path $directory -Force | Out-Null
        $shortcut = $shell.CreateShortcut((Join-Path $directory 'Voxel Project.lnk'))
        $shortcut.TargetPath = Join-Path $destinationPath 'Play.bat'
        $shortcut.WorkingDirectory = $destinationPath
        $shortcut.IconLocation = Join-Path $destinationPath 'voxelproject.exe'
        $shortcut.Save()
    }
}
Write-Host 'Installed. Start Voxel Project from the desktop, or run Play.bat in the install folder.'
