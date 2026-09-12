#!/usr/bin/env powershell

$ErrorActionPreference = 'Stop'

$protocVersion = '25.1'
$protocSha256 = 'b55901fc748d1679f3a803bdc2a920e1897eb02433c501b5a589ea08c4623844'
$protocArchiveName = "protoc-$protocVersion-win64.zip"
$protocUrl = "https://github.com/protocolbuffers/protobuf/releases/download/v$protocVersion/$protocArchiveName"
$isGitHubActions = $env:GITHUB_ACTIONS -eq 'true'

if ($isGitHubActions) {
    $installRoot = Join-Path $env:RUNNER_TEMP "protoc-$protocVersion-$PID"
} else {
    $installRoot = Join-Path $env:LOCALAPPDATA "protoc-$protocVersion"
}
$protocExe = Join-Path $installRoot 'bin/protoc.exe'

if (-not (Test-Path $protocExe)) {
    New-Item -ItemType Directory -Path $installRoot -Force | Out-Null
    $archivePath = Join-Path $installRoot $protocArchiveName
    Invoke-WebRequest -Uri $protocUrl -OutFile $archivePath
    $actualSha256 = (Get-FileHash -Path $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualSha256 -ne $protocSha256) {
        throw "protoc archive SHA-256 mismatch: expected $protocSha256, got $actualSha256"
    }
    Expand-Archive -Path $archivePath -DestinationPath $installRoot -Force
    Remove-Item $archivePath
}

$env:PROTOC = $protocExe
if ($isGitHubActions) {
    (Split-Path $protocExe) | Out-File -FilePath $env:GITHUB_PATH -Encoding utf8 -Append
    "PROTOC=$protocExe" | Out-File -FilePath $env:GITHUB_ENV -Encoding utf8 -Append
} else {
    [Environment]::SetEnvironmentVariable('PROTOC', $protocExe, 'User')
}
