# Builds the companion mod into companion\1.6\Assemblies and companion\1.7\Assemblies (those
# dlls are committed: CI has no game). 1.6 is built against the installed game, 1.7 against the
# reference assemblies from NuGet. Needs the .NET SDK and RimWorld 1.6 installed.
# Usage: .\companion\build.ps1 [-GameDir <RimWorld 1.6 folder>]
param([string]$GameDir)
$ErrorActionPreference = "Stop"

function Find-RimWorld {
    $steam = (Get-ItemProperty "HKCU:\Software\Valve\Steam" -ErrorAction SilentlyContinue).SteamPath
    $libraries = @()
    if ($steam) {
        $libraries += $steam
        $vdf = Join-Path $steam "steamapps\libraryfolders.vdf"
        if (Test-Path $vdf) {
            $libraries += Select-String -Path $vdf -Pattern '"path"\s+"([^"]+)"' |
                ForEach-Object { $_.Matches[0].Groups[1].Value -replace '\\\\', '\' }
        }
    }
    foreach ($lib in $libraries) {
        $dir = Join-Path $lib "steamapps\common\RimWorld"
        if (Test-Path (Join-Path $dir "RimWorldWin64_Data\Managed\Assembly-CSharp.dll")) { return $dir }
    }
    return $null
}

if (-not $GameDir) { $GameDir = Find-RimWorld }
$project = Join-Path $PSScriptRoot "Source\Companion.csproj"
if ($GameDir) {
    $managed = Join-Path $GameDir "RimWorldWin64_Data\Managed"
    if (-not (Test-Path (Join-Path $managed "Assembly-CSharp.dll"))) { throw "No RimWorld 1.6 assemblies in $managed" }
    dotnet build $project -c Release -nologo "-p:RimWorldManaged=$managed"
} else {
    # No RimWorld 1.6 on this machine (1.7 is installed): the 1.6 build uses NuGet's reference assemblies.
    Write-Host "RimWorld 1.6 not found: building 1.6 against Krafs.Rimworld.Ref from NuGet"
    dotnet build $project -c Release -nologo
}
if ($LASTEXITCODE -ne 0) { throw "build failed" }
Remove-Item (Join-Path $PSScriptRoot "Source\obj") -Recurse -Force -ErrorAction SilentlyContinue
Get-Item (Join-Path $PSScriptRoot "1.*\Assemblies\RimWorldModManagerCompanion.dll")
