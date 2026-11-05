@echo off
setlocal
rem ==== Compile et installe Ableton Manager + son serveur MCP ====
set "DEST=%USERPROFILE%\ableton-manager"
set "MCPDIR=%LOCALAPPDATA%\AbletonManager\mcp"

where git >nul 2>&1 || (echo [ERREUR] Git introuvable. & pause & exit /b 1)
where npm >nul 2>&1 || (echo [ERREUR] Node.js introuvable. & pause & exit /b 1)
where cargo >nul 2>&1 || (echo [ERREUR] Rust introuvable ^(https://rustup.rs^). & pause & exit /b 1)

if not exist "%DEST%\.git" (
  git clone https://github.com/atheasstudio/ableton-manager.git "%DEST%" || goto :err
) else (
  cd /d "%DEST%"
  git checkout main && git pull origin main || goto :err
)
cd /d "%DEST%"

echo === Serveur MCP ===
taskkill /f /im ableton-manager-mcp.exe >nul 2>&1
call cargo build --release -p ableton-manager-mcp || goto :err
if not exist "%MCPDIR%" mkdir "%MCPDIR%"
copy /y "%DEST%\target\release\ableton-manager-mcp.exe" "%MCPDIR%\" >nul || goto :err

echo === Branchement dans Claude Desktop ===
powershell -NoProfile -Command ^
  "$f=\"$env:APPDATA\Claude\claude_desktop_config.json\"; New-Item -ItemType Directory -Force (Split-Path $f) | Out-Null;" ^
  "$c= if (Test-Path $f) { Get-Content $f -Raw | ConvertFrom-Json } else { [pscustomobject]@{} };" ^
  "if (-not $c.mcpServers) { $c | Add-Member -Force mcpServers ([pscustomobject]@{}) };" ^
  "$c.mcpServers | Add-Member -Force 'ableton-manager' ([pscustomobject]@{ command = \"$env:LOCALAPPDATA\AbletonManager\mcp\ableton-manager-mcp.exe\" });" ^
  "$c | ConvertTo-Json -Depth 20 | Set-Content -Encoding UTF8 $f"
where claude >nul 2>&1 && call claude mcp add -s user ableton-manager -- "%MCPDIR%\ableton-manager-mcp.exe" >nul 2>&1

echo === Application ===
call npm install || goto :err
call npm run tauri build || goto :err

set "SETUP="
for %%F in ("%DEST%\target\release\bundle\nsis\*-setup.exe") do set "SETUP=%%F"
if not defined SETUP for /r "%DEST%" %%F in (*-setup.exe) do echo %%F | find /i "\bundle\nsis\" >nul && set "SETUP=%%F"
if not defined SETUP (echo [ERREUR] Installeur introuvable. & pause & exit /b 1)
echo Redemarre Claude Desktop pour voir le serveur ableton-manager.
start "" "%SETUP%"
exit /b 0

:err
echo [ERREUR] Une etape a echoue, voir ci-dessus.
pause
exit /b 1
