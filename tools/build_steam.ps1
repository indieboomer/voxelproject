param([switch]$Release)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location $projectRoot
try {
    $cargo = Join-Path $env:USERPROFILE '.cargo/bin/cargo.exe'
    if (!(Test-Path -LiteralPath $cargo)) { $cargo = 'cargo' }
    $buildArgs = @('build', '--features', 'steam')
    $profile = 'debug'
    if ($Release) { $buildArgs += '--release'; $profile = 'release' }
    & $cargo @buildArgs
    if ($LASTEXITCODE -ne 0) { throw 'Steam build failed' }
    $output = Join-Path $projectRoot "target/$profile"
    $dll = Get-ChildItem -Path "$output/build/steamworks-sys-*/out/steam_api64.dll" | Select-Object -First 1
    if (!$dll) { throw 'Steam redistributable DLL was not found' }
    Copy-Item -LiteralPath $dll.FullName -Destination (Join-Path $output 'steam_api64.dll') -Force
    Write-Host "Steam build: $output/voxelproject.exe"
    Write-Host 'Run from the project directory. Sign into Steam; choose Settings > Multiplayer > Steam friends.'
} finally { Pop-Location }
