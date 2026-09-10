param(
    [switch]$DirectOnly,
    [switch]$NoAI,
    [switch]$NoZip,
    [switch]$Offline,
    [switch]$VerifyOnly,
    [string]$RuntimeDirectory,
    [string]$CrtDirectory
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$projectRoot = Split-Path -Parent $PSScriptRoot
if (!$RuntimeDirectory) { $RuntimeDirectory = Join-Path $projectRoot 'llm-runtime' }
$RuntimeDirectory = [IO.Path]::GetFullPath($RuntimeDirectory)
# Keep this name aligned with src/llm_server.rs; other GGUF files are not shipped.
$modelName = 'qwen2.5-coder-7b-instruct-q4_k_m.gguf'
function Require-File([string]$Path) {
    if (!(Test-Path -LiteralPath $Path -PathType Leaf)) { throw "Required file is missing: $Path" }
}
if (!$NoAI) {
    Require-File (Join-Path $RuntimeDirectory "models/$modelName")
    foreach ($name in @('llama-server.exe', 'llama.dll', 'ggml.dll', 'ggml-base.dll')) {
        Require-File (Join-Path $RuntimeDirectory "server/$name")
    }
    if (!(Get-ChildItem -LiteralPath (Join-Path $RuntimeDirectory 'server') -Filter 'ggml-cpu*.dll')) {
        throw 'The AI server bundle needs a CPU backend for machines without GPU acceleration.'
    }
}
# Ship app-local CRT files from Visual Studio's redistributable directory,
# never arbitrary DLLs from System32. This avoids an administrator-only prerequisite installer.
if (!$CrtDirectory) {
    $candidates = @()
    foreach ($base in @($env:ProgramFiles, ${env:ProgramFiles(x86)})) {
        if ($base) {
            $candidates += Get-ChildItem -Path "$base/Microsoft Visual Studio/*/*/VC/Redist/MSVC/*/x64/Microsoft.VC*.CRT" -Directory -ErrorAction SilentlyContinue
        }
    }
    $crt = $candidates | Sort-Object { [version]$_.Parent.Parent.Name } -Descending | Select-Object -First 1
    if (!$crt) { throw 'MSVC x64 redistributable folder not found. Install Visual Studio C++ tools or pass -CrtDirectory.' }
    $CrtDirectory = $crt.FullName
}
foreach ($name in @('vcruntime140.dll', 'vcruntime140_1.dll', 'msvcp140.dll')) {
    Require-File (Join-Path $CrtDirectory $name)
}
foreach ($name in @('Cargo.lock', 'data/crafting.json', 'data/resources.json', 'tools/package/Install.ps1')) {
    Require-File (Join-Path $projectRoot $name)
}
$cargo = Join-Path $env:USERPROFILE '.cargo/bin/cargo.exe'
if (!(Test-Path -LiteralPath $cargo)) { $cargo = (Get-Command cargo -ErrorAction Stop).Source }
if ($VerifyOnly) {
    Write-Host "Package inputs OK. CRT: $CrtDirectory"
    if (!$NoAI) { Write-Host "AI runtime: $RuntimeDirectory; model: $modelName" }
    exit 0
}

Push-Location $projectRoot
try {
    $buildArgs = @('build', '--release', '--locked', '--message-format=json-render-diagnostics')
    if (!$DirectOnly) { $buildArgs += @('--features', 'steam') }
    if ($Offline) { $buildArgs += '--offline' }
    $executable = $null
    $steamDll = $null
    Write-Host 'Building the release executable...'
    & $cargo @buildArgs | ForEach-Object {
        $message = $_ | ConvertFrom-Json
        if ($message.reason -eq 'compiler-artifact' -and $message.target.name -eq 'voxelproject' -and $message.executable) {
            $executable = $message.executable
        }
        if ($message.reason -eq 'build-script-executed' -and $message.package_id -match 'steamworks-sys') {
            $steamDll = Join-Path $message.out_dir 'steam_api64.dll'
        }
    }
    if ($LASTEXITCODE -ne 0 -or !$executable) { throw 'Release build failed; no package was created.' }
    if (!$DirectOnly) { if (!$steamDll) { throw 'Cargo did not report a Steam redistributable.' }; Require-File $steamDll }

    $edition = if ($DirectOnly) { 'direct' } else { 'steam' }
    $content = if ($NoAI) { 'client' } else { 'full' }
    $name = 'VoxelProject-win64-{0}-{1}-{2}' -f $edition, $content, (Get-Date -Format 'yyyyMMdd-HHmmss-fff')
    # Each invocation creates a fresh directory; it never deletes a previous package or user save.
    $output = Join-Path $projectRoot "dist/$name"
    New-Item -ItemType Directory -Path $output | Out-Null
    Copy-Item -LiteralPath $executable -Destination (Join-Path $output 'voxelproject.exe')
    if (!$DirectOnly) { Copy-Item -LiteralPath $steamDll -Destination $output }
    foreach ($directory in @('data', 'modules')) {
        New-Item -ItemType Directory -Path (Join-Path $output $directory) | Out-Null
    }
    foreach ($file in @('data/crafting.json', 'data/resources.json')) {
        Copy-Item -LiteralPath (Join-Path $projectRoot $file) -Destination (Join-Path $output $file)
    }
    Get-ChildItem -LiteralPath (Join-Path $projectRoot 'modules') -Filter '*.lua' -File |
        Copy-Item -Destination (Join-Path $output 'modules')
    Get-ChildItem -LiteralPath (Join-Path $PSScriptRoot 'package') -File | Copy-Item -Destination $output
    Get-ChildItem -LiteralPath $CrtDirectory -Filter '*.dll' -File | Copy-Item -Destination $output
    # Fresh preferences, independent of the developer's settings and nickname.
    $mode = if ($DirectOnly) { 'direct' } else { 'steam' }
    # serde_json expects UTF-8 without a BOM (Windows PowerShell's UTF8 adds one).
    [IO.File]::WriteAllText((Join-Path $output 'settings.json'), (@{multiplayer = @{mode = $mode; steam_app_id = 480}} | ConvertTo-Json -Depth 4))
    if (!$NoAI) {
        Write-Host 'Copying the AI model and server libraries...'
        $serverOutput = Join-Path $output 'llm-runtime/server'
        New-Item -ItemType Directory -Path $serverOutput -Force | Out-Null
        New-Item -ItemType Directory -Path (Join-Path $output 'llm-runtime/models') | Out-Null
        # Keep all server DLLs/backends and supplied notices, excluding logs, archives and unrelated CLI tools.
        Get-ChildItem -LiteralPath (Join-Path $RuntimeDirectory 'server') -File |
            Where-Object { $_.Extension -eq '.dll' -or $_.Name -eq 'llama-server.exe' -or $_.Name -match '^(LICENSE|COPYING|NOTICE)' } |
            Copy-Item -Destination $serverOutput
        Get-ChildItem -LiteralPath $CrtDirectory -Filter '*.dll' -File | Copy-Item -Destination $serverOutput -Force
        Copy-Item -LiteralPath (Join-Path $RuntimeDirectory "models/$modelName") -Destination (Join-Path $output 'llm-runtime/models')
        Get-ChildItem -LiteralPath (Join-Path $RuntimeDirectory 'models') -File |
            Where-Object { $_.Name -match '^(LICENSE|COPYING|NOTICE|README)' } |
            Copy-Item -Destination (Join-Path $output 'llm-runtime/models')
    }
    # Include any locally maintained third-party notices alongside runtime-supplied licenses.
    if (Test-Path -LiteralPath (Join-Path $projectRoot 'licenses')) {
        Copy-Item -LiteralPath (Join-Path $projectRoot 'licenses') -Destination $output -Recurse
    }
    $metadataArgs = @('metadata', '--locked', '--offline', '--format-version', '1', '--filter-platform', 'x86_64-pc-windows-msvc')
    if (!$DirectOnly) { $metadataArgs += @('--features', 'steam') }
    $metadataJson = & $cargo @metadataArgs
    if ($LASTEXITCODE -ne 0) { throw 'Could not collect dependency notices.' }
    $metadata = $metadataJson | ConvertFrom-Json
    $dependencyNotices = Join-Path $output 'licenses/rust-dependencies'
    New-Item -ItemType Directory -Path $dependencyNotices -Force | Out-Null
    $index = @()
    foreach ($package in $metadata.packages) {
        if ($package.name -eq 'voxelproject') { continue }
        $index += "$($package.name) $($package.version): $($package.license) $($package.repository)"
        $noticeFiles = @(Get-ChildItem -LiteralPath (Split-Path -Parent $package.manifest_path) -File |
            Where-Object { $_.Name -match '^(LICENSE|LICENCE|COPYING|NOTICE)' })
        if ($package.license_file) {
            $noticeFiles += Get-Item -LiteralPath (Join-Path (Split-Path -Parent $package.manifest_path) $package.license_file)
        }
        if ($noticeFiles.Count) {
            $noticeOutput = Join-Path $dependencyNotices "$($package.name)-$($package.version)"
            New-Item -ItemType Directory -Path $noticeOutput -Force | Out-Null
            $noticeFiles | Copy-Item -Destination $noticeOutput -Force
        }
    }
    [IO.File]::WriteAllLines((Join-Path $dependencyNotices 'INDEX.txt'), [string[]]$index)
    Write-Host 'Hashing package files...'
    $files = @(Get-ChildItem -LiteralPath $output -File -Recurse | Sort-Object FullName | ForEach-Object {
        @{ path = $_.FullName.Substring($output.Length + 1).Replace('\', '/'); bytes = $_.Length;
           sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash }
    })
    $manifest = @{format = 1; edition = $edition; bundled_ai = !$NoAI; created_utc = [DateTime]::UtcNow.ToString('o'); files = $files}
    [IO.File]::WriteAllText((Join-Path $output 'package-manifest.json'), ($manifest | ConvertTo-Json -Depth 6))
    if (!$NoZip) {
        # Streaming ZipArchive supports ZIP64 and >4 GB GGUF entries, unlike Compress-Archive.
        Add-Type -AssemblyName System.IO.Compression
        $zipPath = "$output.zip"
        Write-Host "Creating ZIP64 archive: $zipPath"
        $stream = [IO.File]::Open($zipPath, [IO.FileMode]::CreateNew)
        try {
            $archive = New-Object IO.Compression.ZipArchive($stream, [IO.Compression.ZipArchiveMode]::Create, $true)
            try {
                foreach ($file in Get-ChildItem -LiteralPath $output -File -Recurse) {
                    $relative = $file.FullName.Substring($output.Length + 1).Replace('\', '/')
                    $entry = $archive.CreateEntry("$name/$relative", [IO.Compression.CompressionLevel]::NoCompression)
                    $entryStream = $entry.Open()
                    try {
                        $inputStream = [IO.File]::OpenRead($file.FullName)
                        try { $inputStream.CopyTo($entryStream) } finally { $inputStream.Dispose() }
                    } finally { $entryStream.Dispose() }
                }
            } finally { $archive.Dispose() }
        } finally { $stream.Dispose() }
        $hash = (Get-FileHash -LiteralPath $zipPath -Algorithm SHA256).Hash
        [IO.File]::WriteAllText("$zipPath.sha256", "$hash  $name.zip`r`n")
        Write-Host "Share: $zipPath"
    }
    Write-Host "Portable/installable folder: $output"
} finally { Pop-Location }
