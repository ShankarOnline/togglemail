# Builds the Microsoft Store package (MSIX) from the Windows release build.
#   .\scripts\msix.ps1 -IdentityName <Package/Identity/Name> -Publisher "CN=..." -PublisherName "<Publisher display name>"
# The three values are on Partner Center > ToggleMail > Product management > Product identity.
# The Store signs the package itself when it publishes it, so this one stays unsigned.
param(
    [Parameter(Mandatory)] [string] $IdentityName,
    [Parameter(Mandatory)] [string] $Publisher,
    [Parameter(Mandatory)] [string] $PublisherName,
    [switch] $NoBuild
)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$conf = Get-Content "$root\src-tauri\tauri.conf.json" -Raw | ConvertFrom-Json
# The Store wants four parts, the last one 0.
$version = "$($conf.version).0"

$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
if (-not $NoBuild) {
    Push-Location "$root\src-tauri"
    # Through cmd: Windows PowerShell treats cargo's progress messages on stderr as errors.
    try { cmd /c "cargo tauri build --no-bundle 2>&1"; if ($LASTEXITCODE) { throw "cargo tauri build failed" } } finally { Pop-Location }
}
$exe = "$root\src-tauri\target\release\togglemail.exe"
if (-not (Test-Path $exe)) { throw "No $exe" }

$makeappx = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\makeappx.exe" | Sort-Object FullName | Select-Object -Last 1
if (-not $makeappx) { throw "makeappx.exe not found: install the Windows SDK" }

$out = "$root\build\msix"
$stage = "$out\stage"
Remove-Item -Recurse -Force $out -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force "$stage\Assets" | Out-Null
Copy-Item $exe "$stage\ToggleMail.exe"
$icons = "$root\src-tauri\icons"
Copy-Item "$icons\StoreLogo.png" "$stage\Assets\StoreLogo.png"
Copy-Item "$icons\Square44x44Logo.png" "$stage\Assets\Square44x44Logo.png"
Copy-Item "$icons\Square150x150Logo.png" "$stage\Assets\Square150x150Logo.png"

$esc = { param($s) [System.Security.SecurityElement]::Escape($s) }
@"
<?xml version="1.0" encoding="utf-8"?>
<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10"
         xmlns:uap="http://schemas.microsoft.com/appx/manifest/uap/windows10"
         xmlns:rescap="http://schemas.microsoft.com/appx/manifest/foundation/windows10/restrictedcapabilities"
         IgnorableNamespaces="uap rescap">
  <Identity Name="$(& $esc $IdentityName)" Publisher="$(& $esc $Publisher)" Version="$version" ProcessorArchitecture="x64" />
  <Properties>
    <DisplayName>ToggleMail</DisplayName>
    <PublisherDisplayName>$(& $esc $PublisherName)</PublisherDisplayName>
    <Logo>Assets\StoreLogo.png</Logo>
    <Description>One email app for all your accounts</Description>
  </Properties>
  <Dependencies>
    <TargetDeviceFamily Name="Windows.Desktop" MinVersion="10.0.17763.0" MaxVersionTested="10.0.26100.0" />
  </Dependencies>
  <Resources>
    <Resource Language="en-us" />
  </Resources>
  <Applications>
    <Application Id="ToggleMail" Executable="ToggleMail.exe" EntryPoint="Windows.FullTrustApplication">
      <uap:VisualElements DisplayName="ToggleMail" Description="One email app for all your accounts"
                          BackgroundColor="transparent" Square150x150Logo="Assets\Square150x150Logo.png"
                          Square44x44Logo="Assets\Square44x44Logo.png" />
    </Application>
  </Applications>
  <Capabilities>
    <Capability Name="internetClient" />
    <rescap:Capability Name="runFullTrust" />
  </Capabilities>
</Package>
"@ | Set-Content -Encoding utf8 "$stage\AppxManifest.xml"

$msix = "$out\ToggleMail_$($version)_x64.msix"
& $makeappx.FullName pack /o /d $stage /p $msix | Out-Null
if ($LASTEXITCODE) { throw "makeappx failed" }
Write-Output "Made $msix ($([math]::Round((Get-Item $msix).Length / 1MB, 1)) MB)"
