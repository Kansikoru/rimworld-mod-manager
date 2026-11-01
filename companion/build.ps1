# Builds the companion mod into companion\Assemblies (that dll is committed: CI has no game).
# Needs the .NET SDK and RimWorld 1.6 installed. Usage: .\companion\build.ps1 [-GameDir <RimWorld folder>]
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
if (-not $GameDir) { throw "RimWorld 1.6 not found: pass -GameDir <RimWorld folder>" }
$managed = Join-Path $GameDir "RimWorldWin64_Data\Managed"
if (-not (Test-Path (Join-Path $managed "Assembly-CSharp.dll"))) { throw "No RimWorld 1.6 assemblies in $managed" }

$project = Join-Path $PSScriptRoot "Source\Companion.csproj"
dotnet build $project -c Release -nologo "-p:RimWorldManaged=$managed"
if ($LASTEXITCODE -ne 0) { throw "build failed" }
Remove-Item (Join-Path $PSScriptRoot "Source\obj") -Recurse -Force -ErrorAction SilentlyContinue
Get-Item (Join-Path $PSScriptRoot "Assemblies\RimWorldModManagerCompanion.dll")
