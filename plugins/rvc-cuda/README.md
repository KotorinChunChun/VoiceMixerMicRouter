# VoiceMixer RVC CUDA Process Plugin

KahoA/KahoB等のRVCモデルをNVIDIA CUDAで推論するProcess Transform Pluginです。既定モデルはKahoAです。

追跡対象の`plugin.json`、Python Worker、ライセンス情報に加え、実行前にステージングスクリプトで次のローカル同梱物を配置します。

```text
runtime/
infer/
configs/
assets/
  hubert/hubert_base.pt
  rmvpe/rmvpe.pt
models/KahoA.pth
models/KahoB.pth
models/Kaho.index
```

大容量のPython/CUDA runtimeとモデルはGit管理外です。次のコマンドで参照元RVCをPlugin配下へ同梱します。

`rmvpe`をF0方式に選択する場合は`assets/rmvpe/rmvpe.pt`が必要です。このAssetもステージングスクリプトが参照元から配置します。

```powershell
pwsh -NoProfile -File .\dev/scripts\stage-rvc-cuda-plugin.ps1
```

既定のRVC参照元は`C:\RVC\RVC20250401Nvidia50`、Kahoモデル参照元は`C:\RVC\rvcgirls_Kaho`です。別の場所を使う場合は`-SourceRoot`または`-KahoRoot`を指定します。推論コード、モデル、Assetだけを更新し、runtimeを再コピーしない場合は`-SkipRuntime`を指定できます。

指定WAVによる実モデル検証:

```powershell
pwsh -NoProfile -File .\dev/scripts\convert-rvc-kahoa.ps1
```

成功すると`plugins/rvc-cuda/output/`へ変換WAVを生成し、遅延、frame数、Peak、RMS、非無音sample数を表示します。

KahoAの画像設定とKahoBの既定設定は、それぞれ次のPresetを使用します。

```powershell
pwsh -NoProfile -File .\dev/scripts\convert-rvc-kahoa.ps1
pwsh -NoProfile -File .\dev/scripts\convert-rvc-kahob.ps1
```

`InputPath`を省略するとファイル選択ダイアログが開きます。Pathを第1引数として渡すこともできるため、ExplorerからPowerShellへ対象ファイルをドラッグして、そのPathを次のように使用できます。

```powershell
pwsh -NoProfile -File .\dev/scripts\convert-rvc-kahoa.ps1 'C:\Audio\input.wav'
pwsh -NoProfile -File .\dev/scripts\convert-rvc-kahob.ps1 'C:\Audio\input.wav'
```

任意のモデル・Parameterを指定する場合は共通Scriptを使用します。

```powershell
pwsh -NoProfile -File .\dev/scripts\convert-rvc-cuda.ps1 `
  -InputPath '.\tests/fixtures/audio\こんにちは。10秒間のマイクテストです.wav' `
  -ModelPath '.\plugins\rvc-cuda\models\KahoA.pth' `
  -IndexPath '.\plugins\rvc-cuda\models\Kaho.index' `
  -Pitch 10 -Formant 0 -F0Method rmvpe -IndexRate 0 `
  -BlockTime 0.25 -CrossfadeTime 0.08 -ExtraTime 5 `
  -RmsMixRate 0.5 -ThresholdDb -58 -CpuWorkers 4
```

共通Scriptは`-UseJit`、`-InputNoiseReduction`、`-OutputNoiseReduction`にも対応します。`-OutputPath`を省略するとモデル名と入力名から`plugins/rvc-cuda/output/`配下へ自動生成します。

任意モデルを指定する例:

```powershell
& '.\plugins\rvc-cuda\runtime\python.exe' '.\dev/scripts\verify-rvc-cuda-plugin.py' `
  --input '.\tests/fixtures/audio\こんにちは。10秒間のマイクテストです.wav' `
  --model 'C:\RVC\rvcgirls_Kaho\KahoB.pth' `
  --index 'C:\RVC\rvcgirls_Kaho\Kaho.index' `
  --threshold-db -50 `
  --input-noise-reduction `
  --output-noise-reduction `
  --output '.\plugins\rvc-cuda\output\KahoB-こんにちは。10秒間のマイクテストです.wav'
```

`index_rate=0`ではIndexを使用しないため、`--index`は省略可能です。`index_rate`を0より大きくする場合は対象モデルと組になるIndexを指定してください。

VoiceMixer本体では、`plugin.json`のParameter Schemaから登録画面とRouting Propertiesを自動生成します。Model/Indexを含む全Parameter、Bypass、既定値復元をGUIで操作できます。`restart_required=true`のModel/Index、Timing、CPU、JITを適用した場合、稼働中なら対象Plugin Nodeだけを自動再起動します。

## Parameter実装状況

| Parameter | 既定値 | 状況 | 反映タイミング・補足 |
| :--- | :--- | :--- | :--- |
| `model` | `models/KahoA.pth` | 実装済み | 起動時。変更後はNode停止・再開始が必要 |
| `index` | `models/Kaho.index` | 実装済み | 起動時。`index_rate=0`では未使用 |
| `pitch` | `0.0` | 実装済み | 実行中に変更可能、単位は半音 |
| `formant` | `0.0` | 実装済み | 実行中に変更可能 |
| `f0_method` | `fcpe` | 実装済み | 実行中に変更可能 |
| `index_rate` | `0.0` | 実装済み | 実行中に変更可能。0より大きい場合はIndex必須 |
| `block_time` | `0.25` | 実装済み | 起動時。内部推論Block秒数 |
| `crossfade_time` | `0.05` | 実装済み | 起動時。SOLA Crossfade秒数 |
| `extra_time` | `2.5` | 実装済み | 起動時。推論Context秒数 |
| `rms_mix_rate` | `0.0` | 実装済み | 実行中に変更可能。参照GUI相当の時間変化RMS Envelope補正 |
| `threshold_db` | `-60.0` | 実装済み | 実行中に変更可能。`-60`はGate無効、`-59`〜`0`でInput Gate |
| `n_cpu` | `4` | 実装済み | 起動時。最大4 Worker |
| `use_jit` | `false` | 実装済み | 起動時 |
| `input_noise_reduction` | `false` | 実装済み | 実行中に変更可能。CUDA stationary spectral gate |
| `output_noise_reduction` | `false` | 実装済み | 実行中に変更可能。CUDA stationary spectral gate |
| `transient_noise_suppression` | `false` | 実装済み | 実行中に変更可能。既存の推論履歴で孤立した短い衝撃音を検出し、SOLAの位置に合わせて変換後の対応区間を抑制 |

短い衝撃音の抑制は、強い部分が短く、エネルギーが集中して減衰する音を対象にします。小さいクリックや最大500msの余韻・複合音も候補にし、継続音の前後250msを保護します。推論へ渡した入力履歴のサンプル位置とSOLA補正量を使い、出力の共有リングに入れる前に適用します。新たな音声待ちや先読みバッファは追加せず、出力フレーム数も変えません。後続音の取得が不足し、減衰も確認できない場合は通します。短い発声との誤判定や、声と重なったクリックの通過はあり得ます。詳細条件は[常駐の現行仕様](../../docs/mic-resident-spec.md)を参照してください。MIC経路には適用されません。

同じRVC変換結果の抑制前後を比較する実モデル試験（再生デバイスへは出力しません）:

```powershell
& .\plugins\rvc-cuda\runtime\python.exe -B .\dev\scripts\verify-rvc-transient.py `
  --settings .\release\MicRouter\mic-settings.json `
  --output-directory .\tmp\rvc-transient-verification
```

生成した短い衝撃音と音声fixtureを使い、抑制前後のWAV、フレーム数、短音区間の減衰、声の区間の変更サンプル数、判定の計算時間を保存します。申告固定遅延の不変と、実際の音響遅延の測定は別です。

利用者が再現した48kHzの録音で検証する場合は、音声デバイスへ再生せず、同じ変換結果の抑制前後を複数の処理タイミングで比較できます。区間にはRVC内部の対応位置のずれを含めるため、対象音の前後に十分な無音余白を含めてください。

```powershell
& .\plugins\rvc-cuda\runtime\python.exe -B .\dev\scripts\verify-rvc-transient-recording.py `
  --settings .\release\MicRouter\mic-settings.json `
  --input .\recordings\noise-and-speech.wav `
  --output-directory .\tmp\rvc-recording-verification --speech 0:15 --noise 25:47
```

現用Workerと同じGPU・精度設定を使います。区間は録音に合わせて指定してください。`--baseline-gate` に旧版の `transient_gate.py` を指定すれば、同じ変換結果へ旧・新両方のゲインを適用して比較できます。録音はこの検証用であり、モデルへの学習や外部送信は行いません。

実行中のModel/Index変更を無停止で再ロードする処理は未実装です。Model/Index、内部Buffer構成、Worker数、JITの変更はProcess再生成時に反映されます。

ProtocolだけをRVC依存なしで検証する場合は、Python Workerへ`--engine passthrough`を指定します。

初期実装はWindows x86_64とNVIDIA CUDA専用です。CUDAが利用できない場合はPrepareを失敗させ、原音へ自動Bypassしません。
