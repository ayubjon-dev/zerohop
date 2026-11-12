# Builds the Windows installer from the Release build, with the version from
# pubspec.yaml. Run from the repository root after `flutter build windows`.
#
# The installer carries the same product name and version as zerohop.exe,
# which code signing requires of every file in a release.

$ErrorActionPreference = 'Stop'

$iscc = Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe'
if (-not (Test-Path $iscc)) {
  choco install innosetup --no-progress --yes
  if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
}

# "0.1.0+1": shown as 0.1.0, numbered 0.1.0.1 as Flutter numbers the program.
$full = (Select-String -Path app/pubspec.yaml -Pattern '^version:\s*(\S+)').Matches[0].Groups[1].Value
$version, $build = $full -split '\+', 2
if (-not $build) { $build = '0' }

& $iscc "/DAppVersion=$version" "/DFullVersion=$full" "/DNumericVersion=$version.$build" `
  app/windows/installer/zerohop.iss
exit $LASTEXITCODE
