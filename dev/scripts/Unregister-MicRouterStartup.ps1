# 自動起動だけを解除する。常駐の終了はトレイ、または Start-MicRouter.ps1 -Mode Stop。
$ErrorActionPreference = 'Stop'
$shortcutPath = Join-Path ([Environment]::GetFolderPath('Startup')) 'VoiceMixer Mic Router.lnk'
if (Test-Path -LiteralPath $shortcutPath) { Remove-Item -LiteralPath $shortcutPath }
Write-Host 'VoiceMixer マイク常駐の自動起動を解除しました。'
