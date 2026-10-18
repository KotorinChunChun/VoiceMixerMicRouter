param([switch]$SkipBuild,[switch]$RequireClean)
$ErrorActionPreference='Stop'
$root=Split-Path (Split-Path $PSScriptRoot)
if($RequireClean -and (git -C $root status --porcelain)){throw '公開ビルドにはコミット済みのクリーンなソースが必要です。'}
$sourceCommit=git -C $root rev-parse HEAD
$version=(Get-Content (Join-Path $root 'tool.json') -Raw|ConvertFrom-Json).version
$target=Join-Path $root 'artifacts/cargo-native'
$binary=Join-Path $target 'x86_64-pc-windows-msvc/release/voicemixer-mic.exe'
$helper=Join-Path $root 'artifacts/maintenance-publish'
if(-not $SkipBuild){
 $oldTarget=$env:CARGO_TARGET_DIR;$oldFlags=$env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS
 try{
  $env:CARGO_TARGET_DIR=$target
  $env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS='-C target-feature=+crt-static'
  & cargo build --locked --manifest-path (Join-Path $root 'src/src-tauri/Cargo.toml') --target x86_64-pc-windows-msvc --release --bin voicemixer-mic -j 4
  if($LASTEXITCODE -ne 0){throw 'ネイティブEXEのビルド失敗'}
 }finally{$env:CARGO_TARGET_DIR=$oldTarget;$env:CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS=$oldFlags}
 & dotnet publish (Join-Path $root 'src/maintenance/MicRouter.Maintenance.csproj') -c Release -r win-x64 --self-contained true -p:PublishSingleFile=false -p:DebugType=None -p:DebugSymbols=false -o $helper --nologo
 if($LASTEXITCODE -ne 0){throw '保守helperの発行失敗'}
}
$info=[Diagnostics.FileVersionInfo]::GetVersionInfo($binary)
if($info.ProductName -ne 'VoiceMixerMicRouter' -or "$($info.FileMajorPart).$($info.FileMinorPart).$($info.FileBuildPart)" -ne $version){throw 'EXEの製品資源と定義が一致しません'}
$run=Join-Path $root ('artifacts/installers/'+[DateTime]::UtcNow.ToString('yyyyMMddTHHmmssZ')+'-'+[Guid]::NewGuid().ToString('N').Substring(0,8))
$payload=Join-Path $run 'payload'
New-Item -ItemType Directory $payload -Force|Out-Null
Copy-Item -LiteralPath $binary -Destination (Join-Path $payload 'VoiceMixerMicRouter.exe')
Copy-Item -LiteralPath $helper -Destination (Join-Path $payload 'maintenance') -Recurse
$options=(Get-Content (Join-Path $helper 'KotorinToys.Maintenance.runtimeconfig.json') -Raw|ConvertFrom-Json).runtimeOptions
if($options.framework -or $options.frameworks -or -not $options.includedFrameworks){throw '保守helperが自己完結型ではありません'}
$assets=Get-Content (Join-Path $root 'src/maintenance/obj/project.assets.json') -Raw|ConvertFrom-Json
foreach($framework in $options.includedFrameworks){
 $packageId=$framework.name.ToLowerInvariant()+'.runtime.win-x64'
 $package=@($assets.packageFolders.PSObject.Properties.Name|ForEach-Object{Join-Path $_ "$packageId/$($framework.version)"}|Where-Object{Test-Path $_})[0]
 if(-not $package){throw 'ランタイム配布通知が見つかりません'}
 $notices=Join-Path $payload "maintenance/runtime-licenses/$($framework.name)"
 New-Item -ItemType Directory $notices -Force|Out-Null
 foreach($name in @('LICENSE.TXT','THIRD-PARTY-NOTICES.TXT')){Copy-Item -LiteralPath (Join-Path $package $name) -Destination $notices}
}
foreach($name in @('tool.json','README.md')){Copy-Item -LiteralPath (Join-Path $root $name) -Destination $payload}
New-Item -ItemType Directory (Join-Path $payload 'docs') -Force|Out-Null
Copy-Item -Path (Join-Path $root 'docs/*') -Destination (Join-Path $payload 'docs') -Recurse
$plugin=Join-Path $payload 'plugins/rvc-cuda'
New-Item -ItemType Directory $plugin -Force|Out-Null
foreach($name in @('plugin.json','rvc_plugin.py','rvc_engine.py','shared_audio.py','spectral_gate.py','transient_gate.py','RVC_LICENSE.txt','README.md')){Copy-Item -LiteralPath (Join-Path $root "plugins/rvc-cuda/$name") -Destination $plugin}
$files=@(Get-ChildItem -LiteralPath $payload -File -Recurse|ForEach-Object{
 $relative=[IO.Path]::GetRelativePath($payload,$_.FullName)
 if($relative -match '(^|[\/])(mic-settings.json|status.json|private|temp|models|runtime)([\/]|$)' -or $_.Extension -in @('.pth','.pt','.index','.pfx','.key','.pem','.pdb')){throw "配布禁止ファイル: $relative"}
 [ordered]@{path=$relative.Replace('\','/');length=$_.Length;sha256=(Get-FileHash $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant()}
})
[ordered]@{sourceCommit=$sourceCommit;files=$files}|ConvertTo-Json -Depth 8|Set-Content (Join-Path $run 'payload-files.json') -Encoding utf8
& (Join-Path $PSScriptRoot 'prepare-inno.ps1')
if($LASTEXITCODE -ne 0){throw 'Inno準備失敗'}
$compiler=Join-Path $root 'artifacts/dependencies/inno-7.1.0/ISCC.exe'
& $compiler --quiet-progress '--define=ProductName=VoiceMixerMicRouter' '--define=ProductAppId=KotorinToys.VoiceMixerMicRouter' "--define=ProductVersion=$version" "--define=Payload=$payload" "--define=OutputDirectory=$run" (Join-Path $root 'installers/product.iss')
if($LASTEXITCODE -ne 0){throw 'インストーラのコンパイル失敗'}
$filename="VoiceMixerMicRouter-$version-setup.exe"
$setup=Join-Path $run $filename
$hash=(Get-FileHash $setup -Algorithm SHA256).Hash.ToLowerInvariant()
if((Get-Item $setup).Length -gt 512MB){throw '本体の配布サイズ上限を超えています'}
[ordered]@{schemaVersion=1;distributionStatus='published';packages=@([ordered]@{id='voice-mixer-mic-router';name='VoiceMixerMicRouter';version=$version;appId='KotorinToys.VoiceMixerMicRouter';file=$filename;sha256=$hash;signature='unsigned-public-build'})}|ConvertTo-Json -Depth 8|Set-Content (Join-Path $run 'VoiceMixerMicRouter-packages.json') -Encoding utf8
"$hash  $filename"|Set-Content (Join-Path $run 'SHA256SUMS.txt') -Encoding utf8
[ordered]@{sourceCommit=$sourceCommit;version=$version;installer=$setup;sha256=$hash;length=(Get-Item $setup).Length}|ConvertTo-Json|Set-Content (Join-Path $run 'build-info.json') -Encoding utf8
Write-Output "インストーラ: $setup"
