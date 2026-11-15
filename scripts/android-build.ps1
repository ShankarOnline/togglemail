# Builds the Android app on Windows. Tauri's own `android build` links the Rust library into the
# Android project with a symbolic link, which Windows only allows in Developer Mode; this script copies
# it instead and runs Gradle itself.
#   .\scripts\android-build.ps1 [-Arch arm64|x86_64] [-Release]   one APK for testing
#   .\scripts\android-build.ps1 -Bundle                            signed release AAB for Google Play (all ABIs)
# Needs: the Android SDK + NDK (C:\Users\<you>\Android\env.ps1), rustup targets for Android, tauri-cli,
# and for -Bundle the upload key settings in ~/Android/keys/togglemail-release.properties.
param([ValidateSet('arm64', 'x86_64')] [string]$Arch = 'arm64', [switch]$Release, [switch]$Bundle)

# Native tools print progress on stderr, which Windows PowerShell treats as errors: check results instead.
$ErrorActionPreference = 'Continue'
. "$env:USERPROFILE\Android\env.ps1"
$env:NDK_HOME = (Get-ChildItem "$env:ANDROID_HOME\ndk" -Directory | Sort-Object Name | Select-Object -Last 1).FullName
$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
$env:CARGO_TARGET_DIR = $null

$root = Resolve-Path "$PSScriptRoot\..\src-tauri"
# Tauri target name, Rust triple, Android ABI folder, Gradle flavour.
$abis = @{
    arm64  = @('aarch64', 'aarch64-linux-android', 'arm64-v8a', 'Arm64')
    arm    = @('armv7', 'armv7-linux-androideabi', 'armeabi-v7a', 'Arm')
    x86    = @('i686', 'i686-linux-android', 'x86', 'X86')
    x86_64 = @('x86_64', 'x86_64-linux-android', 'x86_64', 'X86_64')
}
if ($Bundle) { $Release = $true }
$profile = if ($Release) { 'release' } else { 'debug' }
$variant = if ($Release) { 'Release' } else { 'Debug' }
$build = if ($Bundle) { @('arm64', 'arm', 'x86', 'x86_64') } else { @($Arch) }

# 1. Rust library per ABI (Tauri compiles it, then stops at the symbolic link; that failure is expected),
# 2. copied where Gradle expects it.
foreach ($a in $build) {
    $t, $triple, $abi, $null = $abis[$a]
    Push-Location $root
    $args = @('tauri', 'android', 'build', '--apk', '--target', $t)
    if (-not $Release) { $args += '--debug' }
    & cargo @args 2>&1 | Select-String -Pattern 'Finished|error\[' | ForEach-Object { "$a $($_.Line)" }
    Pop-Location
    $so = "$root\target\$triple\$profile\libtogglemail_lib.so"
    if (-not (Test-Path $so)) { throw "Rust library was not built for ${a}: $so" }
    $jni = "$root\gen\android\app\src\main\jniLibs\$abi"
    New-Item -ItemType Directory -Force $jni | Out-Null
    Copy-Item $so $jni -Force
}

# 3. Package (a clean build: Gradle sometimes reuses stale Kotlin output otherwise).
Push-Location "$root\gen\android"
if ($Bundle) {
    & .\gradlew.bat clean bundleUniversalRelease -x rustBuildUniversalRelease --console=plain 2>&1 |
        Select-String -Pattern 'BUILD|FAILURE|What went wrong' | ForEach-Object { $_.Line }
} else {
    $flavor = $abis[$Arch][3]
    & .\gradlew.bat clean "assemble$flavor$variant" -x "rustBuild$flavor$variant" --console=plain 2>&1 |
        Select-String -Pattern 'BUILD|FAILURE|What went wrong' | ForEach-Object { $_.Line }
}
Pop-Location
Get-ChildItem "$root\gen\android\app\build\outputs" -Recurse -Include '*.apk', '*.aab' | Select-Object -ExpandProperty FullName
