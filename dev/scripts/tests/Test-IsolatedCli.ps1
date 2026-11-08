param([Parameter(Mandatory)][string]$Executable, [string[]]$ProtectedSettings = @())
$ErrorActionPreference = 'Stop'
$exe = (Resolve-Path -LiteralPath $Executable).Path
$root = Split-Path (Split-Path (Split-Path $PSScriptRoot))
$fixture = Join-Path $PSScriptRoot 'fixtures/isolated-cli.json'
$version = (Get-Content -LiteralPath (Join-Path $root 'tool.json') -Raw | ConvertFrom-Json).version
$run = Join-Path $root ("dev/history/v$version/logs/isolated-cli-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $run | Out-Null

function Check([bool]$Ok, [string]$Label) {
    if (-not $Ok) { throw $Label }
    "PASS: $Label"
}
function Read-ResidentPids {
    @(Get-Process VoiceMixerMicRouter,voicemixer-mic -ErrorAction SilentlyContinue |
        Sort-Object Id | ForEach-Object Id) -join ','
}
function Invoke-Scenario([string]$Label, [string]$File, [int]$ExpectedExit, [string[]]$Extra = @()) {
    $stdout = Join-Path $run "$Label.json"
    $stderr = Join-Path $run "$Label.log"
    $arguments = @('--test-scenario', ('"' + $File + '"')) + $Extra
    $process = Start-Process -FilePath $exe -ArgumentList $arguments -WindowStyle Hidden -PassThru `
        -RedirectStandardOutput $stdout -RedirectStandardError $stderr
    if (-not $process.WaitForExit(15000)) { throw "隔離CLIが時間内に終了しません: $Label" }
    if ($process.ExitCode -ne $ExpectedExit) {
        throw "終了コード不一致 $Label : $($process.ExitCode) / $(Get-Content -LiteralPath $stdout -Raw)"
    }
    $report = Get-Content -LiteralPath $stdout -Raw -Encoding utf8 | ConvertFrom-Json
    if ($report.version -ne $version -or $report.backend -ne 'isolated' -or $report.schema_version -ne 1) {
        throw "JSONの契約が一致しません: $Label"
    }
    return $report
}
function Read-Hash([string]$File) {
    if (Test-Path -LiteralPath $File) { (Get-FileHash -LiteralPath $File).Hash }
}

$pids = Read-ResidentPids
$before = @{}
foreach ($file in @($fixture) + $ProtectedSettings) { $before[$file] = Read-Hash $file }

$report = Invoke-Scenario 'success' $fixture 0
$s = $report.steps
Check ($report.ok -and $s.Count -eq 35) 'シナリオ全35操作が成功'
Check ($s[1].state.draft_changed -and -not $s[4].state.draft_changed) '下書きと適用を分離'
Check ($s[2].detail.choice -eq 7 -and $s[2].detail.device_id -eq 'fixture-render' -and
       $s[2].detail.frames -eq 4800 -and $s[2].detail.peak -gt 0 -and -not $s[2].detail.played) '効果音の試聴波形と下書きのデバイス'
Check ($s[5].state.mode -eq 'muted' -and $s[5].state.startup_mode -eq 'speaker_rvc') 'RVC無効で再起動時は送信停止'
Check ($s[7].detail.choice -eq 7 -and ($s[7].state.outputs.saved -join ',') -eq 'True,False,False') '出力切替と割り当てられた効果音'
Check (($s[8].state.outputs.effective -join ',') -eq 'False,True,False' -and
       ($s[9].state.outputs.effective -join ',') -eq 'False,True,True' -and
       ($s[10].state.outputs.effective -join ',') -eq 'False,False,True' -and
       ($s[11].state.outputs.effective -join ',') -eq 'True,False,False') '同時押下、押下中の排他、解放で出力を復元'
Check (($s[13].state.outputs.saved -join ',') -eq 'True,True,True') '出力1〜3を切替可能'
Check (-not $s[14].state.overlay.visible -and $s[15].state.overlay.visible -and
       $s[16].state.overlay.remaining_seconds -eq 1 -and -not $s[17].state.overlay.visible -and
       $s[18].state.overlay.visible -and $s[19].state.overlay.mode -eq 'always') 'オーバーレイの切替、4秒の期限、再表示'
Check ($s[22].state.mode -eq 'rvc' -and $s[24].state.mode -eq 'speaker_rvc' -and
       -not $s[25].state.noise_suppression -and $s[26].state.mode -eq 'direct') 'RVCモード、ノイズ、解放の状態遷移'
$beforeReset = $s[32].detail
Check ($beforeReset.applied.hotkeys[2].key -eq 'F4' -and $beforeReset.draft.hotkeys[2].key -eq 'None' -and
       $beforeReset.applied.rvc_parameters.pitch -eq 3 -and $beforeReset.draft.rvc_parameters.pitch -eq 10) 'キーとRVCのリセットは下書きだけ変更'
$reset = $s[-1].detail.applied
Check (-not $reset.rvc_enabled -and $reset.microphone_id -eq 'default://console' -and
       $reset.cable_id -eq 'default://console' -and $reset.voice_input_id -eq '' -and $reset.monitor_id -eq '' -and
       $reset.sound_device_id -eq 'default://console' -and ($reset.switch_sounds -join ',') -eq '1,2,3,4,5,6,7' -and
       $reset.hotkeys[13].ctrl_side -eq 'left' -and $reset.hotkeys[13].shift_side -eq 'left') '4タブのリセットを適用して既定値を確認'

$negative = Join-Path $run 'negative-input.json'
$invalidSteps = @(
    @{op='settings_patch'; values=@{rvc_parameters=@{pitch=500}}}
    @{op='settings_apply'}
    @{op='inspect'}
    @{op='output_hold'; output=2; pressed=$true}
    @{op='output_toggle'; output=0}
    @{op='sound_preview'; choice=8}
    @{op='mode'; mode='rvc'}
    @{op='settings_patch'; values=@{plugin_directory='C:/invalid'}}
    @{op='overlay_show'}
)
@{schema_version=1; steps=$invalidSteps} | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $negative -Encoding utf8
$errorReport = Invoke-Scenario 'negative' $negative 1
$errors = @($errorReport.steps | Where-Object { -not $_.ok })
Check (-not $errorReport.ok -and ($errors.step -join ',') -eq '2,4,5,6,7,8') '操作エラーのJSONと終了コード1'
Check ($errorReport.steps[2].detail.applied.rvc_parameters.pitch -eq 10 -and
       $errorReport.steps[2].detail.draft.rvc_parameters.pitch -eq 500 -and $errorReport.steps[-1].ok) '適用失敗時は設定を保持し後続も報告'
$bad = Join-Path $run 'bad-input.json'
'{not json}' | Set-Content -LiteralPath $bad -Encoding utf8
$badReport = Invoke-Scenario 'bad-json' $bad 2
$conflict = Invoke-Scenario 'mixed-arguments' $fixture 2 @('--stop')
Check ($badReport.error -and $badReport.steps.Count -eq 0 -and $conflict.error -and $conflict.steps.Count -eq 0) '不正JSONと通常起動引数の併用を実行前に拒否'
Check ((Read-ResidentPids) -eq $pids) '他の常駐のPIDを維持'
foreach ($file in $before.Keys) {
    Check ((Read-Hash $file) -eq $before[$file]) ('設定または入力ファイルを変更しない ' + (Split-Path $file -Leaf))
}
"結果: $run"
