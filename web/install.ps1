# Installs the latest Strongtalk for Windows:
#   irm https://volland.github.io/strongtalk/install.ps1 | iex
# Puts strongtalk.exe in %LOCALAPPDATA%\Programs\Strongtalk and adds it to your user PATH.
$ErrorActionPreference = 'Stop'
$repo = 'Volland/strongtalk'
$dir = Join-Path $env:LOCALAPPDATA 'Programs\Strongtalk'
$url = "https://github.com/$repo/releases/latest/download/strongtalk-windows-x64.zip"
$tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("strongtalk-" + [guid]::NewGuid())
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
Write-Host "Downloading $url"
Invoke-WebRequest -Uri $url -OutFile (Join-Path $tmp 'st.zip') -UseBasicParsing
Expand-Archive -Path (Join-Path $tmp 'st.zip') -DestinationPath $tmp -Force
New-Item -ItemType Directory -Force -Path $dir | Out-Null
Copy-Item (Join-Path $tmp 'strongtalk.exe') (Join-Path $dir 'strongtalk.exe') -Force
Remove-Item -Recurse -Force $tmp
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (-not ($userPath -split ';' | Where-Object { $_ -eq $dir })) {
  [Environment]::SetEnvironmentVariable('Path', ($userPath.TrimEnd(';') + ';' + $dir), 'User')
  $env:Path += ";$dir"
  Write-Host "Added $dir to your PATH (open a new terminal to use it)."
}
& (Join-Path $dir 'strongtalk.exe') --version
Write-Host 'Try:  strongtalk          (terminal workspace)'
Write-Host '      strongtalk web      (the environment, in your browser)'
