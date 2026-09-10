$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$root = Join-Path (Split-Path -Parent $PSScriptRoot) ('target/package-tests/' + [Guid]::NewGuid().ToString('N'))
$source = Join-Path $root 'package with spaces'
$destination = Join-Path $root 'installed game'
New-Item -ItemType Directory -Path $source -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'package/Install.ps1') -Destination $source
[IO.File]::WriteAllText((Join-Path $source 'voxelproject.exe'), 'test payload')
[IO.File]::WriteAllText((Join-Path $source 'settings.json'), '{}')
$files = @(Get-ChildItem -LiteralPath $source -File | ForEach-Object {
    @{path=$_.Name; bytes=$_.Length; sha256=(Get-FileHash -LiteralPath $_.FullName).Hash}
})
function Write-Manifest($Entries) {
    [IO.File]::WriteAllText((Join-Path $source 'package-manifest.json'), (@{files=@($Entries)} | ConvertTo-Json -Depth 5))
}
function Assert($Condition, [string]$Message) { if (!$Condition) { throw $Message } }
Write-Manifest $files
$installer = Join-Path $source 'Install.ps1'
& $installer -Destination $destination -NoShortcuts
Assert (Test-Path -LiteralPath (Join-Path $destination 'voxelproject.exe')) 'Fresh install failed'
New-Item -ItemType Directory -Path (Join-Path $destination 'saves') | Out-Null
[IO.File]::WriteAllText((Join-Path $destination 'saves/world.bin'), 'personal world')
[IO.File]::WriteAllText((Join-Path $destination 'settings.json'), '{"personal":true}')
& $installer -Destination $destination -NoShortcuts
Assert ([IO.File]::ReadAllText((Join-Path $destination 'settings.json')) -eq '{"personal":true}') 'Upgrade overwrote preferences'
Assert ([IO.File]::ReadAllText((Join-Path $destination 'saves/world.bin')) -eq 'personal world') 'Upgrade overwrote save'

[IO.File]::WriteAllText((Join-Path $source 'voxelproject.exe'), 'test payloaD')
$rejected = $false
try { & $installer -Destination $destination -NoShortcuts } catch { $rejected = $_ -match 'damaged' }
Assert $rejected 'Same-length corrupted payload was accepted'
Assert ([IO.File]::ReadAllText((Join-Path $destination 'voxelproject.exe')) -eq 'test payload') 'Corrupt install modified existing game'

Write-Manifest @(@{path='../outside.txt'; bytes=0; sha256=''})
$rejected = $false
try { & $installer -Destination $destination -NoShortcuts } catch { $rejected = $_ -match 'Invalid package path' }
Assert $rejected 'Path traversal was accepted'
Write-Host 'PASS: install with spaces, upgrade preservation, corruption rejection, path validation.'
