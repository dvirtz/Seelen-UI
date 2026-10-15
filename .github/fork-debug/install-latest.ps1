#requires -Version 5.1
<#
.SYNOPSIS
Downloads, verifies, installs and launches the latest successful Fork debug build.
.EXAMPLE
./.github/fork-debug/install-latest.ps1 -WhatIf
.EXAMPLE
./.github/fork-debug/install-latest.ps1 -RunId 38632678315 -NoLaunch
#>
[CmdletBinding(SupportsShouldProcess)]
param(
    [ValidatePattern('^[\w.-]+/[\w.-]+$')]
    [string] $Repository = 'dvirtz/Seelen-UI',
    [ValidateRange(0, [long]::MaxValue)]
    [long] $RunId = 0,
    [switch] $NoLaunch
)

function Get-GitHubHeaders {
    $token = $env:GH_TOKEN
    if (!$token) { $token = $env:GITHUB_TOKEN }
    if (!$token) {
        # Reuse Git's credential helper without printing credentials.
        $request = "protocol=https`nhost=github.com`n`n"
        $credentials = @{}
        $response = $request | git credential fill
        if ($LASTEXITCODE) { throw 'GitHub authentication failed. Sign in through Git, or set GH_TOKEN.' }
        foreach ($line in $response) {
            $parts = $line -split '=', 2
            if ($parts.Length -eq 2) { $credentials[$parts[0]] = $parts[1] }
        }
        $token = $credentials['password']
    }
    if (!$token) { throw 'A GitHub login is required to download Actions artifacts. Set GH_TOKEN or sign in through Git.' }
    return @{
        Authorization = "Bearer $token"
        Accept = 'application/vnd.github+json'
        'X-GitHub-Api-Version' = '2022-11-28'
        'User-Agent' = 'Seelen-fork-debug-installer'
    }
}

function Find-DebugArtifact($ApiRoot, $Headers, [long] $RequestedRun) {
    $page = 1
    do {
        if ($RequestedRun) {
            $runs = @(Invoke-RestMethod "$ApiRoot/actions/runs/$RequestedRun" -Headers $Headers)
        } else {
            $result = Invoke-RestMethod "$ApiRoot/actions/workflows/fork-debug.yml/runs?status=success&per_page=50&page=$page" -Headers $Headers
            $runs = @($result.workflow_runs)
        }
        foreach ($run in $runs) {
            if ($run.conclusion -ne 'success' -or ($run.path -split '@', 2)[0] -ne '.github/workflows/fork-debug.yml') { continue }
            $artifactPage = 1
            do {
                $result = Invoke-RestMethod "$ApiRoot/actions/runs/$($run.id)/artifacts?per_page=100&page=$artifactPage" -Headers $Headers
                $artifacts = @($result.artifacts)
                foreach ($artifact in ($artifacts | Sort-Object created_at -Descending)) {
                    if (!$artifact.expired -and $artifact.name -match '^seelen-debug-x64-([a-f0-9]{40})$') {
                        return @{ Run = $run; Artifact = $artifact; SourceSha = $Matches[1] }
                    }
                }
                $artifactPage++
            } while ($artifacts.Count -eq 100)
        }
        $page++
    } while (!$RequestedRun -and $runs.Count -eq 50)
    throw 'No successful Fork debug build with an unexpired debug artifact was found.'
}

function Get-ArtifactPath([string] $Root, [string] $RelativePath) {
    if ([IO.Path]::IsPathRooted($RelativePath) -or $RelativePath.Contains(':')) {
        throw "Unsafe artifact path: $RelativePath"
    }
    $prefix = [IO.Path]::GetFullPath($Root).TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar
    $path = [IO.Path]::GetFullPath((Join-Path $Root $RelativePath))
    if (!$path.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Artifact path escapes its destination: $RelativePath"
    }
    return $path
}

function Expand-DebugArtifact([string] $Zip, [string] $Stage) {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [IO.Compression.ZipFile]::OpenRead($Zip)
    try {
        $paths = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
        foreach ($entry in $archive.Entries) {
            $path = Get-ArtifactPath $Stage $entry.FullName
            if (!$paths.Add($path)) { throw "Duplicate artifact path: $($entry.FullName)" }
            # Reject symbolic links instead of extracting links outside the installation.
            if ((($entry.ExternalAttributes -shr 16) -band 0xF000) -eq 0xA000) {
                throw "Artifact contains a symbolic link: $($entry.FullName)"
            }
        }
    } finally { $archive.Dispose() }
    [IO.Compression.ZipFile]::ExtractToDirectory($Zip, $Stage)
}

function Test-DebugArtifact([string] $Stage, [string] $SourceSha) {
    $manifest = Join-Path $Stage 'ARTIFACT_SHA256SUMS.txt'
    $entries = @(Get-Content -LiteralPath $manifest)
    $seen = [Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
    $index = 0
    try {
        foreach ($line in $entries) {
            if ($line -notmatch '^([a-fA-F0-9]{64})  (.+)$') { throw 'Malformed artifact checksum manifest.' }
            $expected = $Matches[1]
            $relative = $Matches[2]
            $path = Get-ArtifactPath $Stage $relative
            if (!$seen.Add($path)) { throw "Duplicate checksum entry: $relative" }
            if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash -ne $expected) {
                throw "Artifact checksum mismatch: $relative"
            }
            $index++
            if ($index % 100 -eq 0) {
                Write-Progress -Activity 'Verifying debug files' -Status "$index / $($entries.Count)" -PercentComplete ($index * 100 / $entries.Count)
            }
        }
    } finally { Write-Progress -Activity 'Verifying debug files' -Completed }
    foreach ($file in Get-ChildItem -LiteralPath $Stage -Recurse -File) {
        if ($file.FullName -ne $manifest -and !$seen.Contains($file.FullName)) {
            throw "Artifact contains an unlisted file: $($file.FullName)"
        }
    }
    foreach ($name in @('seelen-ui.exe', 'slu.exe', 'slu-service.exe', 'sluhk.dll',
        'seelen_ui.pdb', 'slu.pdb', 'slu_service.pdb', 'sluhk.pdb',
        'SHA256SUMS', 'SHA256SUMS.sig', 'BUILD_INFO.json', 'COMMIT_SHA.txt')) {
        if (!(Test-Path -LiteralPath (Join-Path $Stage $name) -PathType Leaf)) { throw "Missing debug file: $name" }
    }
    foreach ($name in @('static', 'frontend')) {
        if (!(Test-Path -LiteralPath (Join-Path $Stage $name) -PathType Container)) { throw "Missing debug directory: $name" }
    }
    $info = Get-Content -LiteralPath (Join-Path $Stage 'BUILD_INFO.json') -Raw | ConvertFrom-Json
    $commit = (Get-Content -LiteralPath (Join-Path $Stage 'COMMIT_SHA.txt') -Raw).Trim()
    if ($info.sourceSha -ne $SourceSha -or $commit -ne $SourceSha -or $info.target -ne 'x86_64-pc-windows-msvc') {
        throw 'Artifact metadata does not match the selected source SHA and Windows x64 target.'
    }
    return $info
}

$ErrorActionPreference = 'Stop'
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
$workspace = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '../..'))
$target = Join-Path $workspace 'target'
$destination = Join-Path $target 'fork-debug'
$headers = Get-GitHubHeaders
$apiRoot = "https://api.github.com/repos/$Repository"
$selected = Find-DebugArtifact $apiRoot $headers $RunId
Write-Host "Build: $($selected.Run.html_url)"
Write-Host "Source: $($selected.SourceSha)"
Write-Host ('Download: {0:N0} MB' -f ($selected.Artifact.size_in_bytes / 1MB))
if (!$PSCmdlet.ShouldProcess($destination, 'Download, verify and install debug build (back up settings and previous build)')) { return }

$running = @(Get-Process -Name 'seelen-ui', 'slu-service' -ErrorAction SilentlyContinue)
if ($running.Count) {
    Write-Host 'Quit Seelen through Settings > Extras > Exit. Allow its helper service to stop.'
    Read-Host 'Press Enter after Seelen has closed' | Out-Null
    if (@(Get-Process -Name 'seelen-ui', 'slu-service' -ErrorAction SilentlyContinue).Count) {
        throw 'Seelen or its service is still running. Close it normally, then rerun this script.'
    }
}

New-Item -ItemType Directory -Force -Path $target | Out-Null
$unique = (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + [guid]::NewGuid().ToString('N').Substring(0, 8)
$stage = Join-Path $target "fork-debug-stage-$unique"
$zip = Join-Path $target "fork-debug-download-$unique.zip"
$backup = Join-Path $target "fork-debug-backup-$unique"
try {
    Write-Host 'Downloading artifact...'
    # HttpWebRequest drops Authorization when following the redirect to artifact storage.
    Invoke-WebRequest "$apiRoot/actions/artifacts/$($selected.Artifact.id)/zip" -Headers $headers -UseBasicParsing -OutFile $zip
    $digest = $selected.Artifact.digest
    if ($digest -match '^sha256:([a-fA-F0-9]{64})$') {
        if ((Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash -ne $Matches[1]) {
            throw 'Downloaded ZIP does not match the GitHub artifact digest.'
        }
    }
    Write-Host 'Extracting artifact (many resource files; this can take several minutes)...'
    Expand-DebugArtifact $zip $stage
    $info = Test-DebugArtifact $stage $selected.SourceSha
    Write-Host "Verified source branch: $($info.branch)"
    # Another instance may have started while downloading.
    if (@(Get-Process -Name 'seelen-ui', 'slu-service' -ErrorAction SilentlyContinue).Count) {
        throw 'Seelen started during the download. Close it before installing.'
    }
    New-Item -ItemType Directory -Path $backup | Out-Null
    foreach ($location in @(
        @{ Root = $env:APPDATA; Name = 'Roaming'; Files = @('settings.json', 'settings_by_app.yml', 'settings_shortcuts.json') },
        @{ Root = $env:LOCALAPPDATA; Name = 'Local'; Files = @('workspaces2.json', 'workspaces3.json') }
    )) {
        $settings = Join-Path $location.Root 'com.seelen.seelen-ui'
        $saved = Join-Path $backup $location.Name
        New-Item -ItemType Directory -Path $saved | Out-Null
        foreach ($name in $location.Files) {
            $file = Join-Path $settings $name
            if (Test-Path -LiteralPath $file -PathType Leaf) { Copy-Item -LiteralPath $file -Destination $saved }
        }
    }
    $previous = Join-Path $backup 'build'
    if (Test-Path -LiteralPath $destination) { Move-Item -LiteralPath $destination -Destination $previous }
    try { Move-Item -LiteralPath $stage -Destination $destination }
    catch {
        if (!(Test-Path -LiteralPath $destination) -and (Test-Path -LiteralPath $previous)) {
            Move-Item -LiteralPath $previous -Destination $destination
        }
        throw
    }
    Write-Host "Installed: $destination"
    Write-Host "Settings and previous-build backup: $backup"
    if (!$NoLaunch) {
        Write-Host 'Launching Seelen. Approve its normal helper-service UAC prompt if requested.'
        $app = Start-Process -FilePath (Join-Path $destination 'seelen-ui.exe') -WorkingDirectory $destination -WindowStyle Normal -PassThru
        Start-Sleep -Seconds 10
        $app.Refresh()
        if ($app.HasExited) {
            throw "Seelen exited during startup. Inspect $env:LOCALAPPDATA\com.seelen.seelen-ui\logs\Seelen UI.log. The verified build remains installed."
        }
        Write-Host "Seelen started (PID $($app.Id))."
    }
} finally {
    if (Test-Path -LiteralPath $zip) { Remove-Item -LiteralPath $zip -Force }
    # Only remove this invocation's staging directory under this checkout's target.
    $resolvedStage = [IO.Path]::GetFullPath($stage)
    if ($resolvedStage -ne (Join-Path $target "fork-debug-stage-$unique")) { throw 'Unexpected staging cleanup path.' }
    if (Test-Path -LiteralPath $resolvedStage) { Remove-Item -LiteralPath $resolvedStage -Recurse -Force }
}
