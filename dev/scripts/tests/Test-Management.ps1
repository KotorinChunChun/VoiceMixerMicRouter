param([Parameter(Mandatory)][string]$Executable,[Parameter(Mandatory)][string]$Configuration)
$ErrorActionPreference='Stop'
if(Get-Process voicemixer-mic,VoiceMixerMicRouter -ErrorAction SilentlyContinue){throw '利用中の常駐を通常終了してから隔離試験を実行してください。'}
$fixture=Get-Content -LiteralPath $Configuration -Raw|ConvertFrom-Json
if($fixture.last_mode -ne 'muted' -or $fixture.rvc_enabled -or $fixture.cable_enabled -or $fixture.voice_input_enabled -or $fixture.monitoring -or @($fixture.hotkeys|Where-Object enabled).Count){throw '送信停止・全出力OFF・RVC/キー無効の隔離設定だけで試験できます。'}
Add-Type @"
using System;using System.Text;using System.Runtime.InteropServices;
public static class Mic118Window {
 [DllImport("user32.dll",CharSet=CharSet.Unicode)]public static extern IntPtr FindWindow(string c,string t);
 [DllImport("user32.dll")]public static extern bool IsWindowVisible(IntPtr h);
 [DllImport("user32.dll")]public static extern IntPtr GetDlgItem(IntPtr h,int id);
 [DllImport("user32.dll",EntryPoint="SendMessageW",CharSet=CharSet.Unicode)]public static extern IntPtr SendText(IntPtr h,uint m,IntPtr w,string text);
 [DllImport("user32.dll")]public static extern bool PostMessage(IntPtr h,uint m,IntPtr w,IntPtr l);
}
"@
$sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$pipeName="KotorinToys.$sid.voice-mixer-mic-router"
function Request($value){
 $client=[IO.Pipes.NamedPipeClientStream]::new('.', $pipeName,[IO.Pipes.PipeDirection]::InOut,([IO.Pipes.PipeOptions]::Asynchronous -bor [IO.Pipes.PipeOptions]::CurrentUserOnly))
 try{
  $client.Connect(2000)
  $writer=[IO.StreamWriter]::new($client,[Text.UTF8Encoding]::new($false),1024,$true)
  $reader=[IO.StreamReader]::new($client,[Text.UTF8Encoding]::new($false),$false,1024,$true)
  $writer.AutoFlush=$true
  $line=if($value -is [string]){$value}else{$value|ConvertTo-Json -Compress}
  $writer.WriteLine($line)
  $reply=$reader.ReadLineAsync().WaitAsync([TimeSpan]::FromSeconds(3)).GetAwaiter().GetResult()
  return $reply|ConvertFrom-Json
 }finally{if($writer){$writer.Dispose()};if($reader){$reader.Dispose()};$client.Dispose()}
}
function WaitReady {
 $deadline=[DateTime]::Now.AddSeconds(15)
 do{try{return Request 'status'}catch{Start-Sleep -Milliseconds 100}}while([DateTime]::Now -lt $deadline)
 throw '管理窓口の起動時間切れ'
}
function Check([bool]$ok,[string]$label){if(-not$ok){throw $label};Write-Output "PASS: $label"}
$running=$null
try{
 foreach($origin in 'host_manual','direct_task','via_kotorintoys'){
  $running=Start-Process -FilePath $Executable -ArgumentList @('--config',('"'+$Configuration+'"'),'--startup','--launch-origin',$origin) -WindowStyle Hidden -PassThru
  $status=WaitReady
  Check ($status.protocolVersion -eq 2 -and $status.ready -and $status.version -eq '1.0.19') 'CurrentUserOnlyから管理契約2へ実接続'
  Check ($status.pid -eq $running.Id -and $status.launchOrigin -eq $origin) "PIDと起動経路 $origin"
  $overlay=[Mic118Window]::FindWindow('VoiceMixerMicOverlayV1',[NullString]::Value)
  Check (-not[Mic118Window]::IsWindowVisible($overlay)) '--startupでオーバーレイを無表示'
  $wrong=Request @{command='stop';expectedPid=($running.Id+1);requestId='wrong'}
  Check ($wrong.result -eq 'refused' -and $wrong.requestId -eq 'wrong') '別PIDへの停止を拒否'
  $unknown=Request @{command='shortcuts';expectedPid=$running.Id}
  Check ($unknown.result -eq 'unsupported') '未対応操作を成功としない'
  if($origin -eq 'host_manual'){
   $shown=Request @{command='show';expectedPid=$running.Id}
   $window=[Mic118Window]::FindWindow('VoiceMixerMicSettingsV1',[NullString]::Value)
   $shown|ConvertTo-Json -Compress|Write-Output
   Write-Output ('設定HWND: '+$window)
   Check ($shown.result -eq 'completed' -and [Mic118Window]::IsWindowVisible($window)) '実設定画面の表示完了'
   $edit=[Mic118Window]::GetDlgItem($window,240)
   [void][Mic118Window]::SendText($edit,12,[IntPtr]0,'未保存テスト')
   $stop=Request @{command='stop';expectedPid=$running.Id}
   Check ($stop.result -eq 'refused' -and -not$running.HasExited) '未保存設定画面を保護して正常停止を拒否'
   [void][Mic118Window]::PostMessage($window,273,[IntPtr]2,[IntPtr]0)
   Start-Sleep -Milliseconds 250
  }
  $stop=Request @{command='stop';expectedPid=$running.Id}
  Check ($stop.result -eq 'accepted') '停止受理を応答'
  Check ($running.WaitForExit(12000)) '受理後に音声ワーカーと実プロセスを正常終了'
  Check ($running.ExitCode -eq 0) '正常終了コード'
  $running=$null
 }
}finally{
 if($running -and -not$running.HasExited){
  $window=[Mic118Window]::FindWindow('VoiceMixerMicSettingsV1',[NullString]::Value)
  if($window -ne [IntPtr]0){[void][Mic118Window]::PostMessage($window,273,[IntPtr]2,[IntPtr]0);Start-Sleep -Milliseconds 200}
  try{Request @{command='stop';expectedPid=$running.Id}|Out-Null;[void]$running.WaitForExit(12000)}catch{}
 }
}
