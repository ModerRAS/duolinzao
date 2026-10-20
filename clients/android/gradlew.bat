@echo off
rem 最小 gradlew.bat：优先系统 gradle，其次已缓存的 8.10.2，最后下载。
setlocal
set DIST_VER=8.10.2
where gradle >nul 2>&1
if %ERRORLEVEL%==0 (
  gradle %*
  exit /b %ERRORLEVEL%
)
for /d %%D in ("%USERPROFILE%\.gradle\wrapper\dists\gradle-%DIST_VER%-bin\*") do (
  if exist "%%D\gradle-%DIST_VER%\bin\gradle.bat" (
    "%%D\gradle-%DIST_VER%\bin\gradle.bat" %*
    exit /b %ERRORLEVEL%
  )
)
echo downloading gradle %DIST_VER% ...
set TMPDIR=%TEMP%\dlz-gradle-%RANDOM%
mkdir "%TMPDIR%"
powershell -NoProfile -Command "Invoke-WebRequest -OutFile '%TMPDIR%\gradle.zip' 'https://services.gradle.org/distributions/gradle-%DIST_VER%-bin.zip'"
powershell -NoProfile -Command "Expand-Archive -Path '%TMPDIR%\gradle.zip' -DestinationPath '%TMPDIR%'"
"%TMPDIR%\gradle-%DIST_VER%\bin\gradle.bat" %*
