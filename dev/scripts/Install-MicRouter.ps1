param(
    [string]$Directory = (Join-Path (Split-Path -Parent (Split-Path -Parent $PSScriptRoot)) 'release/MicRouter'),
    [switch]$SkipBuild,
    [switch]$NoStartup,
    [switch]$NoStart
)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$Directory = [IO.Path]::GetFullPath($Directory)
$manifest = Join-Path $projectRoot 'src/src-tauri/Cargo.toml'
$metadata = & cargo metadata --no-deps --format-version 1 --manifest-path $manifest | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'Cargoのビルド出力先を取得できません。' }
$build = Join-Path $metadata.target_directory 'release/voicemixer-mic.exe'
if (-not $SkipBuild) {
    & cargo build --manifest-path $manifest --release --bin voicemixer-mic
    if ($LASTEXITCODE -ne 0) { throw '常駐アプリのビルドに失敗しました。' }
}
if (-not (Test-Path -LiteralPath $build)) { throw "ビルド成果物がありません: $build" }
New-Item -ItemType Directory -Path $Directory -Force | Out-Null
$executable = Join-Path $Directory 'voicemixer-mic.exe'
$config = Join-Path $Directory 'mic-settings.json'
# ロック中の実行ファイルを強制終了・上書きしない。
Copy-Item -LiteralPath $build -Destination $executable -Force
if (-not (Test-Path -LiteralPath $config)) {
    $process = Start-Process -FilePath $executable -ArgumentList @('--config', ('"' + $config + '"'), '--init', ('"' + (Join-Path $projectRoot 'plugins') + '"')) -WindowStyle Hidden -Wait -PassThru
    if ($process.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $config)) { throw '設定生成に失敗しました。既定の実マイクとVB-CABLEの接続を確認してください。' }
}
Copy-Item -LiteralPath (Join-Path $projectRoot 'docs/mic-resident.md') -Destination (Join-Path $Directory '使い方.md') -Force
if (-not $NoStartup) {
    $startup = [Environment]::GetFolderPath('Startup')
    $shortcutPath = Join-Path $startup 'VoiceMixer Mic Router.lnk'
    $shell = New-Object -ComObject WScript.Shell
    try {
        $shortcut = $shell.CreateShortcut($shortcutPath)
        $shortcut.TargetPath = $executable
        $shortcut.Arguments = '--config "' + $config + '"'
        $shortcut.WorkingDirectory = $Directory
        $shortcut.Description = 'MIC／MIC-RVC／SPK-RVC → 配信用出力（トレイで切替）'
        $shortcut.WindowStyle = 7
        $shortcut.Save()
        [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($shortcut)
    } finally { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($shell) }
    Write-Host "ログイン時の自動起動を登録しました: $shortcutPath"
}
if (-not $NoStart) { & (Join-Path $PSScriptRoot 'Start-MicRouter.ps1') -Directory $Directory }
Write-Host "配備先: $Directory"
Write-Host "設定: $config"
