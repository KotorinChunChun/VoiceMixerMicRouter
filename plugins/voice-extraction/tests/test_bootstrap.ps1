# ネットワーク・既存uvを使わず、不正キャッシュが起動前に拒否されることを検証する。
$ErrorActionPreference = 'Stop'
$packageRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$testParent = [IO.Path]::GetFullPath((Join-Path $packageRoot '.cache/bootstrap-tests'))
$testRoot = Join-Path $testParent ([guid]::NewGuid().ToString())
[IO.Directory]::CreateDirectory((Join-Path $testRoot '.cache/bootstrap')) | Out-Null
$previousTaskPath = $env:PATH
$previousPythonDir = $env:UV_PYTHON_INSTALL_DIR
$previousCacheDir = $env:UV_CACHE_DIR
try {
    Copy-Item -LiteralPath (Join-Path $packageRoot 'setup-runtime.ps1') -Destination (Join-Path $testRoot 'setup-runtime.ps1')
    [IO.File]::WriteAllBytes((Join-Path $testRoot '.cache/bootstrap/uv-0.11.24.zip'), [byte[]](1,2,3,4))
    $env:PATH = "$PSHOME;C:\Windows\system32;C:\Windows"
    if (Get-Command uv -ErrorAction SilentlyContinue) { throw '試験PATHにuvが含まれています' }
    $rejected = $false
    try { & (Join-Path $testRoot 'setup-runtime.ps1') -AcceptDownload -BootstrapOnly }
    catch {
        if ($_.Exception.Message -notmatch 'キャッシュ内のuv配布物が不正') { throw }
        $rejected = $true
    }
    if (-not $rejected) { throw '不正なZIPが拒否されませんでした' }
    if (Test-Path (Join-Path $testRoot '.cache/bootstrap/uv-0.11.24.exe')) { throw '不正なZIPから実行ファイルが作成されました' }
    Write-Output '成功: uv無しPATHで不正ZIPを実行前に拒否'
} finally {
    $env:PATH = $previousTaskPath
    $env:UV_PYTHON_INSTALL_DIR = $previousPythonDir
    $env:UV_CACHE_DIR = $previousCacheDir
    $resolved = [IO.Path]::GetFullPath($testRoot)
    if (-not $resolved.StartsWith($testParent + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw '試験フォルダーの削除範囲が不正です' }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}
