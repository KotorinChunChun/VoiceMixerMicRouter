param(
    [ValidateSet('Last', 'Direct', 'Rvc', 'SpeakerRvc', 'Muted', 'Toggle', 'Stop', 'Settings', 'ReleaseRvc')]
    [string]$Mode = 'Last',
    [string]$Directory = (Join-Path (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)) 'release/MicRouter')
)
$ErrorActionPreference = 'Stop'
$executable = Join-Path $Directory 'voicemixer-mic.exe'
$config = Join-Path $Directory 'mic-settings.json'
if (-not (Test-Path -LiteralPath $executable) -or -not (Test-Path -LiteralPath $config)) {
    throw '先に pwsh -File .\dev\scripts\Install-MicRouter.ps1 を実行してください。'
}
$arguments = @('--config', ('"' + $config + '"'))
switch ($Mode) {
    'Direct' { $arguments += @('--mode', 'direct') }
    'Rvc' { $arguments += @('--mode', 'rvc') }
    'SpeakerRvc' { $arguments += @('--mode', 'speaker_rvc') }
    'Muted' { $arguments += @('--mode', 'muted') }
    'Toggle' { $arguments += '--toggle' }
    'Stop' { $arguments += '--stop' }
    'Settings' { $arguments += '--settings' }
    'ReleaseRvc' { $arguments += '--release-rvc' }
}
Start-Process -FilePath $executable -ArgumentList $arguments -WorkingDirectory $Directory -WindowStyle Hidden
