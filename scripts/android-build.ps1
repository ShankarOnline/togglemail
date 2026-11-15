# Builds the Android app on Windows. Tauri's own `android build` links the Rust library into the
# Android project with a symbolic link, which Windows only allows in Developer Mode; this script copies
# it instead and runs Gradle itself.
#   .\scripts\android-build.ps1 [-Arch arm64|x86_64] [-Release]
# Needs: the Android SDK + NDK (C:\Users\<you>\Android\env.ps1), rustup targets for Android, tauri-cli.
param([ValidateSet('arm64', 'x86_64')] [string]$Arch = 'arm64', [switch]$Release)

# Native tools print progress on stderr, which Windows PowerShell treats as errors: check results instead.
$ErrorActionPreference = 'Continue'
. "$env:USERPROFILE\Android\env.ps1"
$env:NDK_HOME = (Get-ChildItem "$env:ANDROID_HOME\ndk" -Directory | Sort-Object Name | Select-Object -Last 1).FullName
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
$env:CARGO_TARGET_DIR = $null

$root = Resolve-Path "$PSScriptRoot\..\src-tauri"
$target = @{ arm64 = 'aarch64'; x86_64 = 'x86_64' }[$Arch]
$triple = @{ arm64 = 'aarch64-linux-android'; x86_64 = 'x86_64-linux-android' }[$Arch]
$abi = @{ arm64 = 'arm64-v8a'; x86_64 = 'x86_64' }[$Arch]
$profile = if ($Release) { 'release' } else { 'debug' }
$flavor = @{ arm64 = 'Arm64'; x86_64 = 'X86_64' }[$Arch]
$variant = if ($Release) { 'Release' } else { 'Debug' }

# 1. Rust library (Tauri compiles it, then stops at the symbolic link; that failure is expected).
Push-Location $root
$args = @('tauri', 'android', 'build', '--apk', '--target', $target)
if (-not $Release) { $args += '--debug' }
& cargo @args 2>&1 | Select-String -Pattern 'Finished|error\[' | ForEach-Object { $_.Line }
Pop-Location
$so = "$root\target\$triple\$profile\libtogglemail_lib.so"
if (-not (Test-Path $so)) { throw "Rust library was not built: $so" }

# 2. Copy it where Gradle expects it.
$jni = "$root\gen\android\app\src\main\jniLibs\$abi"
New-Item -ItemType Directory -Force $jni | Out-Null
Copy-Item $so $jni -Force

# 3. Package (a clean build: Gradle sometimes reuses stale Kotlin output otherwise).
Push-Location "$root\gen\android"
& .\gradlew.bat clean "assemble$flavor$variant" -x "rustBuild$flavor$variant" --console=plain 2>&1 |
    Select-String -Pattern 'BUILD|FAILURE|What went wrong' | ForEach-Object { $_.Line }
Pop-Location
Get-ChildItem "$root\gen\android\app\build\outputs\apk" -Recurse -Filter '*.apk' | Select-Object -ExpandProperty FullName
