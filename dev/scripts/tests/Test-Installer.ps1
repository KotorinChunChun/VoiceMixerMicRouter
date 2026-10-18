param([Parameter(Mandatory)][string]$Installer)
$ErrorActionPreference='Stop'
$root=Split-Path (Split-Path (Split-Path $PSScriptRoot))
if(Get-Process voicemixer-mic,VoiceMixerMicRouter -ErrorAction SilentlyContinue){throw '通常の常駐を終了してから隔離導入試験を実行してください。'}
$reg='HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\KotorinToys.VoiceMixerMicRouter_is1'
if(Test-Path $reg){throw '既存の正式導入を検出したため、隔離試験を開始しません。'}
$scratch=Join-Path $root ('artifacts/installer-tests/'+[Guid]::NewGuid().ToString('N'))
$app=Join-Path $scratch 'app'
New-Item -ItemType Directory $scratch -Force|Out-Null
$originalLocal=$env:LOCALAPPDATA
$hostSettings=Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) 'KotorinToys/settings.json'
$hostBefore=if(Test-Path $hostSettings){(Get-FileHash $hostSettings).Hash}else{''}
$env:LOCALAPPDATA=Join-Path $scratch 'isolated-user'
$config=Join-Path $env:LOCALAPPDATA 'VoiceMixerMicRouter/mic-settings.json'
$status=Join-Path (Split-Path $config) 'status.json'
$exe=Join-Path $app 'VoiceMixerMicRouter.exe'
$sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$name="KotorinToys.$sid.voice-mixer-mic-router"
$checks=[Collections.Generic.List[string]]::new()
function Check([bool]$ok,[string]$label){if(-not$ok){throw $label};$checks.Add($label);Write-Output "PASS: $label"}
function Request($value){
 $pipe=[IO.Pipes.NamedPipeClientStream]::new('.',$name,[IO.Pipes.PipeDirection]::InOut,([IO.Pipes.PipeOptions]::Asynchronous -bor [IO.Pipes.PipeOptions]::CurrentUserOnly))
 try{
  $pipe.Connect(2000)
  $writer=[IO.StreamWriter]::new($pipe,[Text.UTF8Encoding]::new($false),1024,$true);$writer.AutoFlush=$true
  $reader=[IO.StreamReader]::new($pipe,[Text.UTF8Encoding]::new($false),$false,1024,$true)
  $line=if($value -is [string]){$value}else{$value|ConvertTo-Json -Compress}
  $writer.WriteLine($line)
  return ($reader.ReadLineAsync().WaitAsync([TimeSpan]::FromSeconds(3)).GetAwaiter().GetResult()|ConvertFrom-Json)
 }finally{if($writer){$writer.Dispose()};if($reader){$reader.Dispose()};$pipe.Dispose()}
}
function Setup([string]$label){
 $log=Join-Path $scratch "$label.log"
 $proc=Start-Process -FilePath $Installer -ArgumentList @('/VERYSILENT','/SUPPRESSMSGBOXES','/SP-','/NORESTART','/NOICONS',('/DIR="'+$app+'"'),('/LOG="'+$log+'"')) -WindowStyle Hidden -PassThru
 if(-not$proc.WaitForExit(120000)){throw 'インストーラの完了時間切れ'}
 return $proc.ExitCode
}
function Ready {
 $deadline=[DateTime]::Now.AddSeconds(18)
 do{try{$reply=Request 'status';if($reply.ready){return $reply}}catch{};Start-Sleep -Milliseconds 100}while([DateTime]::Now -lt $deadline)
 throw '管理窓口の準備時間切れ'
}
function WaitRunning {
 $deadline=[DateTime]::Now.AddSeconds(15)
 do{if(Test-Path $status){$state=Get-Content $status -Raw|ConvertFrom-Json;if($state.status.phase -eq 'running'){return $state}};Start-Sleep -Milliseconds 100}while([DateTime]::Now -lt $deadline)
 throw ('モデル未配置で起動できません: '+($state.status|ConvertTo-Json -Compress -Depth 4))
}
Add-Type @"
using System;using System.Runtime.InteropServices;
public static class InstallerMicWindow {
 [DllImport("user32.dll",CharSet=CharSet.Unicode)]public static extern IntPtr FindWindow(string c,string t);
 [DllImport("user32.dll")]public static extern bool PostMessage(IntPtr h,uint m,IntPtr w,IntPtr l);
}
"@
$running=$null
try{
 Check ((Setup 'fresh') -eq 0) 'サイレント新規インストール'
 Check (Test-Path $reg) 'HKCUの正式導入登録'
 $info=[Diagnostics.FileVersionInfo]::GetVersionInfo($exe)
 Check ($info.ProductName -eq 'VoiceMixerMicRouter' -and $info.FileVersion -eq '1.0.18') '導入EXEの製品名・版'
 Check ((Get-Content "$app/tool.json" -Raw|ConvertFrom-Json).appId -eq 'KotorinToys.VoiceMixerMicRouter') 'tool.jsonの製品識別'
 Check (-not(Test-Path "$app/plugins/rvc-cuda/runtime") -and -not(Test-Path "$app/plugins/rvc-cuda/models/KahoA.pth")) 'モデルとPython/CUDAを含まない基本配布'
 $running=Start-Process -FilePath $exe -ArgumentList @('--startup','--launch-origin','host_manual') -WindowStyle Hidden -PassThru
 $reply=Ready
 Check ($reply.pid -eq $running.Id -and $reply.version -eq '1.0.18') '導入EXEと管理応答のPID・版一致'
 $state=WaitRunning
 Check ($state.status.mode -eq 'muted') 'モデル未配置の初回も正常な送信停止で稼働'
 Check (Test-Path $config) '隔離利用者データ領域の初回設定生成'
 $first=Get-Content $config -Raw|ConvertFrom-Json
 Check (-not$first.rvc_enabled -and -not$first.cable_enabled -and -not$first.voice_input_enabled -and -not$first.monitoring) '初回のRVC・全出力OFF'
 $stopped=Request @{command='stop';expectedPid=$running.Id}
 Check ($stopped.result -eq 'accepted' -and $running.WaitForExit(12000)) '導入EXEの正常停止'
 $running=$null
 $first.output_labels[0]='保持テスト';foreach($key in $first.hotkeys){$key.enabled=$false}
 $first|ConvertTo-Json -Depth 30|Set-Content $config -Encoding utf8
 $settingsHash=(Get-FileHash $config).Hash
 $marker=Join-Path $app 'plugins/rvc-cuda/models/user-model.index'
 Set-Content $marker '利用者追加モデルの保持検証用・音声モデルではありません' -Encoding utf8
 $markerHash=(Get-FileHash $marker).Hash
 $running=Start-Process -FilePath $exe -ArgumentList @('--startup','--launch-origin','host_manual') -WindowStyle Hidden -PassThru
 $reply=Ready
 $shown=Request @{command='show';expectedPid=$running.Id}
 Check ($shown.result -eq 'completed') '更新前の設定表示'
 $beforeExe=(Get-FileHash $exe).Hash
 Check ((Setup 'blocked-update') -ne 0 -and -not$running.HasExited) '設定編集中の更新を拒否し強制終了しない'
 Check ((Get-FileHash $exe).Hash -eq $beforeExe) '拒否時にEXEを書き換えない'
 $window=[InstallerMicWindow]::FindWindow('VoiceMixerMicSettingsV1',[NullString]::Value)
 [void][InstallerMicWindow]::PostMessage($window,273,[IntPtr]2,[IntPtr]0)
 Start-Sleep -Milliseconds 250
 Check ((Setup 'retained-update') -eq 0) '正常停止を伴う同版の保持付き更新'
 Check ($running.WaitForExit(10000)) '更新前プロセスの実終了'
 $running=$null
 Check ((Get-FileHash $config).Hash -eq $settingsHash) '更新で利用者設定のハッシュ保持'
 Check ((Get-FileHash $marker).Hash -eq $markerHash) '更新で利用者追加モデルを保持'
 $running=Start-Process -FilePath $exe -ArgumentList @('--startup','--launch-origin','via_kotorintoys') -WindowStyle Hidden -PassThru
 $reply=Ready
 Check ($reply.launchOrigin -eq 'via_kotorintoys') '更新後の管理起動'
 $uninstaller=Join-Path $app 'unins000.exe'
 $remove=Start-Process -FilePath $uninstaller -ArgumentList @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART',('/LOG="'+(Join-Path $scratch 'uninstall.log')+'"')) -WindowStyle Hidden -PassThru
 Check ($remove.WaitForExit(120000) -and $remove.ExitCode -eq 0) '稼働中から正常停止してアンインストール'
 Check ($running.WaitForExit(10000)) '削除対象プロセスの実終了'
 $running=$null
 # Innoのアンインストールはコピー元プロセスの終了直後に後始末が続く場合がある。
 $until=[DateTime]::Now.AddSeconds(10)
 while((Test-Path $reg) -and [DateTime]::Now -lt $until){Start-Sleep -Milliseconds 100}
 Check (-not(Test-Path $reg) -and -not(Test-Path $exe)) '正式登録・製品EXEの削除完了'
 Check ((Get-FileHash $config).Hash -eq $settingsHash) '削除後も個人設定を保持'
 Check ((Get-FileHash $marker).Hash -eq $markerHash) '削除後も利用者追加モデルを保持'
 $hostAfter=if(Test-Path $hostSettings){(Get-FileHash $hostSettings).Hash}else{''}
 Check ($hostAfter -eq $hostBefore) '未登録のKotorinToys設定を作成・変更しない'
 [ordered]@{success=$true;checks=$checks;installerHash=(Get-FileHash $Installer).Hash;scratch=$scratch}|ConvertTo-Json -Depth 6|Set-Content (Join-Path $scratch 'result.json') -Encoding utf8
 Write-Output "導入試験の結果: $scratch"
}finally{
 if($running -and -not$running.HasExited){
  $window=[InstallerMicWindow]::FindWindow('VoiceMixerMicSettingsV1',[NullString]::Value)
  if($window -ne [IntPtr]0){[void][InstallerMicWindow]::PostMessage($window,273,[IntPtr]2,[IntPtr]0);Start-Sleep -Milliseconds 200}
  try{Request @{command='stop';expectedPid=$running.Id}|Out-Null;[void]$running.WaitForExit(12000)}catch{}
 }
 $env:LOCALAPPDATA=$originalLocal
}
