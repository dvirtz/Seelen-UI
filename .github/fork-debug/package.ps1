$ErrorActionPreference = 'Stop'
$output = Join-Path $PWD 'target/x86_64-pc-windows-msvc/debug'
$stage = [System.IO.Path]::GetFullPath((Join-Path $PWD 'target/fork-debug'))
$expectedStage = [System.IO.Path]::GetFullPath("$PWD/target/fork-debug")
if ($stage -ne $expectedStage) { throw 'Artifact destination is outside the expected workspace path' }
if (Test-Path -LiteralPath $stage) { Remove-Item -LiteralPath $stage -Recurse -Force }
New-Item -ItemType Directory -Force -Path $stage | Out-Null

foreach ($name in @('seelen-ui', 'slu', 'slu-service', 'sluhk')) {
    $extension = if ($name -eq 'sluhk') { 'dll' } else { 'exe' }
    $binary = Join-Path $output "$name.$extension"
    $symbols = Join-Path $output "$($name.Replace('-', '_')).pdb"
    if (!(Test-Path $binary) -or !(Test-Path $symbols)) {
        throw "Missing binary or matching PDB: $name"
    }
    if ((Get-AuthenticodeSignature $binary).Status -ne 'NotSigned') {
        throw "Expected unsigned binary: $binary"
    }
    Copy-Item -LiteralPath $binary, $symbols -Destination $stage
}

Copy-Item -LiteralPath "$output/SHA256SUMS", "$output/SHA256SUMS.sig" -Destination $stage
Copy-Item -LiteralPath 'src/static' -Destination "$stage/static" -Recurse
Copy-Item -LiteralPath 'dist' -Destination "$stage/frontend" -Recurse
Copy-Item -LiteralPath 'LICENSE' -Destination $stage

$maps = @(Get-ChildItem "$stage/frontend" -Recurse -Filter '*.js.map')
if (!$maps.Count) { throw 'Frontend source maps were not generated' }
foreach ($map in $maps) {
    $content = Get-Content -LiteralPath $map.FullName -Raw | ConvertFrom-Json
    if (!$content.sourcesContent.Count) { throw "Missing embedded sources: $map" }
}

Set-Content -LiteralPath "$stage/COMMIT_SHA.txt" -Value $env:SOURCE_SHA -Encoding utf8
@{
    sourceSha = $env:SOURCE_SHA
    branch = $env:SOURCE_BRANCH
    toolingSha = $env:TOOLS_SHA
    sourceRoot = "$PWD"
    target = 'x86_64-pc-windows-msvc'
    profile = 'dev: debug=full, opt-level=0, strip=none, debug-assertions=true'
    runUrl = "$env:GITHUB_SERVER_URL/$env:GITHUB_REPOSITORY/actions/runs/$env:GITHUB_RUN_ID"
    rustc = ((& rustc --version) -join '')
} | ConvertTo-Json | Set-Content -LiteralPath "$stage/BUILD_INFO.json" -Encoding utf8

# Preserve the exact generated-input differences, if any, alongside the base SHA.
git diff --binary HEAD | Set-Content -LiteralPath "$stage/GENERATED_INPUTS.diff" -Encoding utf8
Get-ChildItem $stage -Recurse -File | ForEach-Object {
    "$((Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash)  $($_.FullName.Substring($stage.Length + 1))"
} | Set-Content -LiteralPath "$stage/ARTIFACT_SHA256SUMS.txt" -Encoding utf8
