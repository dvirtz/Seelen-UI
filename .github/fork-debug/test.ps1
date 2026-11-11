#requires -Version 5.1
# Offline regression checks; no downloads, installation, or running-app changes.
$ErrorActionPreference = 'Stop'

function Assert($Condition, [string] $Message) {
    if (!$Condition) { throw $Message }
}

function Assert-Throws([scriptblock] $Action, [string] $Pattern) {
    try { & $Action } catch {
        Assert ($_.Exception.Message -like $Pattern) "Unexpected error: $_"
        return
    }
    throw "Expected an error matching: $Pattern"
}

# Load only function declarations, leaving the installer's entry point untouched.
$errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile(
    (Join-Path $PSScriptRoot 'install-latest.ps1'), [ref]$null, [ref]$errors
)
Assert (!$errors.Count) 'Installer has syntax errors'
foreach ($function in $ast.FindAll({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] }, $false)) {
    . ([scriptblock]::Create($function.Extent.Text))
}
$packageAst = [Management.Automation.Language.Parser]::ParseFile(
    (Join-Path $PSScriptRoot 'package.ps1'), [ref]$null, [ref]$errors
)
Assert (!$errors.Count) 'Packaging script has syntax errors'

$sha = 'a' * 40
function Invoke-RestMethod($Uri, $Headers) {
    if ($Uri -like '*/artifacts?*') {
        return @{ artifacts = @(
            @{ name = "seelen-debug-x64-$sha"; expired = $false },
            @{ name = "seelen-release-x64-$sha"; expired = $true },
            @{ name = "seelen-release-x64-$sha"; expired = $false }
        ) }
    }
    $run = @{ id = 123; conclusion = 'success'; path = '.github/workflows/fork-build.yml' }
    if ($Uri -like '*/actions/runs/123') { return $run }
    return @{ workflow_runs = @($run) }
}

foreach ($profile in @('debug', 'release')) {
    foreach ($runId in @(0, 123)) {
        $selected = Find-ForkArtifact 'https://offline.invalid' @{} $runId $profile
        Assert ($selected.Artifact.name -eq "seelen-$profile-x64-$sha") 'Selected wrong profile'
        Assert (!$selected.Artifact.expired) 'Selected expired artifact'
        Assert ($selected.SourceSha -eq $sha) 'Lost source SHA'
    }
}

$fixture = Join-Path ([IO.Path]::GetTempPath()) ("seelen-fork-test-" + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $fixture | Out-Null
try {
    foreach ($name in @('seelen-ui.exe', 'slu.exe', 'slu-service.exe', 'sluhk.dll',
        'seelen_ui.pdb', 'slu.pdb', 'slu_service.pdb', 'sluhk.pdb', 'SHA256SUMS', 'SHA256SUMS.sig')) {
        Set-Content -LiteralPath (Join-Path $fixture $name) -Value 'fixture'
    }
    foreach ($name in @('static', 'frontend')) {
        New-Item -ItemType Directory -Path (Join-Path $fixture $name) | Out-Null
    }
    Set-Content -LiteralPath (Join-Path $fixture 'COMMIT_SHA.txt') -Value $sha

    function Write-FixtureManifest {
        $entries = @(Get-ChildItem -LiteralPath $fixture -Recurse -File |
            Where-Object Name -ne 'ARTIFACT_SHA256SUMS.txt' | ForEach-Object {
                "$((Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash)  $($_.FullName.Substring($fixture.Length + 1))"
            })
        Set-Content -LiteralPath (Join-Path $fixture 'ARTIFACT_SHA256SUMS.txt') -Value $entries
    }

    foreach ($profile in @('debug', 'release')) {
        $info = @{ sourceSha = $sha; target = 'x86_64-pc-windows-msvc'; buildProfile = $profile }
        $info | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $fixture 'BUILD_INFO.json')
        Write-FixtureManifest
        $verified = Test-ForkArtifact $fixture $sha $profile
        Assert ($verified.buildProfile -eq $profile) 'Rejected valid artifact'
        $otherProfile = if ($profile -eq 'debug') { 'release' } else { 'debug' }
        Assert-Throws { Test-ForkArtifact $fixture $sha $otherProfile } '*build profile*'
        Assert-Throws { Test-ForkArtifact $fixture ('b' * 40) $profile } '*metadata*'
    }

    # Existing downloads did not contain buildProfile.
    $info.Remove('buildProfile')
    $info.profile = 'dev: debug=full, opt-level=0, strip=none, debug-assertions=true'
    $info | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $fixture 'BUILD_INFO.json')
    Write-FixtureManifest
    [void](Test-ForkArtifact $fixture $sha 'debug')
    Assert-Throws { Test-ForkArtifact $fixture $sha 'release' } '*build profile*'
    Set-Content -LiteralPath (Join-Path $fixture 'slu.exe') -Value 'damaged'
    Assert-Throws { Test-ForkArtifact $fixture $sha 'debug' } '*checksum mismatch*'
    Assert-Throws { Get-ArtifactPath $fixture '../escape' } '*escapes*'
} finally {
    $resolved = [IO.Path]::GetFullPath($fixture)
    $expected = Join-Path ([IO.Path]::GetTempPath()) ([IO.Path]::GetFileName($fixture))
    if ($resolved -ne $expected -or [IO.Path]::GetFileName($resolved) -notlike 'seelen-fork-test-*') {
        throw 'Unexpected test cleanup path'
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
Write-Host 'Fork installer regression checks passed.'
