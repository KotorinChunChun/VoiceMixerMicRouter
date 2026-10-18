param(
    [string]$SourceRoot = 'C:\RVC\RVC20250401Nvidia50',
    [string]$KahoRoot = 'C:\RVC\rvcgirls_Kaho',
    [string]$DestinationRoot = '',
    [switch]$SkipRuntime
)

$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
if ([string]::IsNullOrWhiteSpace($DestinationRoot)) {
    $DestinationRoot = Join-Path $projectRoot 'plugins\rvc-cuda'
}

$sourcePath = [System.IO.Path]::GetFullPath($SourceRoot)
$kahoPath = [System.IO.Path]::GetFullPath($KahoRoot)
$destinationPath = [System.IO.Path]::GetFullPath($DestinationRoot)
if (-not (Test-Path -LiteralPath $sourcePath -PathType Container)) {
    throw "RVC参照元が存在しません: $sourcePath"
}
if (-not (Test-Path -LiteralPath $kahoPath -PathType Container)) {
    throw "Kahoモデル配置元が存在しません: $kahoPath"
}
if (-not $destinationPath.StartsWith([System.IO.Path]::GetFullPath($projectRoot), [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "配置先はVoiceMixerプロジェクト配下である必要があります: $destinationPath"
}

$requiredSources = @(
    'runtime\python.exe',
    'infer\lib\rtrvc.py',
    'configs\config.py',
    'assets\hubert\hubert_base.pt',
    'assets\rmvpe\rmvpe.pt',
    'LICENSE'
)
foreach ($relativePath in $requiredSources) {
    $candidate = Join-Path $sourcePath $relativePath
    if (-not (Test-Path -LiteralPath $candidate -PathType Leaf)) {
        throw "必要なRVCファイルがありません: $candidate"
    }
}
$requiredKahoFiles = @('KahoA.pth', 'KahoB.pth', 'Kaho.index')
foreach ($fileName in $requiredKahoFiles) {
    $candidate = Join-Path $kahoPath $fileName
    if (-not (Test-Path -LiteralPath $candidate -PathType Leaf)) {
        throw "必要なKahoモデルファイルがありません: $candidate"
    }
}

New-Item -ItemType Directory -Path $destinationPath -Force | Out-Null
if (-not $SkipRuntime) {
    Copy-Item -LiteralPath (Join-Path $sourcePath 'runtime') -Destination $destinationPath -Recurse -Force
}
Copy-Item -LiteralPath (Join-Path $sourcePath 'infer') -Destination $destinationPath -Recurse -Force
Copy-Item -LiteralPath (Join-Path $sourcePath 'configs') -Destination $destinationPath -Recurse -Force

$hubertDestination = Join-Path $destinationPath 'assets\hubert'
$rmvpeDestination = Join-Path $destinationPath 'assets\rmvpe'
$modelDestination = Join-Path $destinationPath 'models'
New-Item -ItemType Directory -Path $hubertDestination -Force | Out-Null
New-Item -ItemType Directory -Path $rmvpeDestination -Force | Out-Null
New-Item -ItemType Directory -Path $modelDestination -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $sourcePath 'assets\hubert\hubert_base.pt') -Destination $hubertDestination -Force
Copy-Item -LiteralPath (Join-Path $sourcePath 'assets\rmvpe\rmvpe.pt') -Destination $rmvpeDestination -Force
Copy-Item -LiteralPath (Join-Path $kahoPath 'KahoA.pth') -Destination (Join-Path $modelDestination 'KahoA.pth') -Force
Copy-Item -LiteralPath (Join-Path $kahoPath 'KahoB.pth') -Destination (Join-Path $modelDestination 'KahoB.pth') -Force
Copy-Item -LiteralPath (Join-Path $kahoPath 'Kaho.index') -Destination (Join-Path $modelDestination 'Kaho.index') -Force
Copy-Item -LiteralPath (Join-Path $sourcePath 'LICENSE') -Destination (Join-Path $destinationPath 'RVC_LICENSE.txt') -Force

$pythonPath = if ($SkipRuntime) {
    Join-Path $sourcePath 'runtime\python.exe'
} else {
    Join-Path $destinationPath 'runtime\python.exe'
}
& $pythonPath --version
& $pythonPath -c "import torch; print('cuda_available=' + str(torch.cuda.is_available())); print('cuda_device=' + (torch.cuda.get_device_name(0) if torch.cuda.is_available() else 'none'))"
if ($LASTEXITCODE -ne 0) {
    throw '同梱PythonのCUDA検査に失敗しました'
}

Write-Output "RVC CUDA Pluginを配置しました: $destinationPath"
Write-Output "Python: $pythonPath"
