# RVC CUDAプラグイン

RVCのPythonワーカーとパラメーター定義を管理します。NVIDIA CUDAに対応するPyTorch・RVC実行環境と、利用許諾のあるモデルが別途必要です。上流RVCのライセンスはRVC_LICENSE.txtに保持します。

モデル・実行環境・取得済み上流コード・生成音声はGitに含めません。

```text
rvc-cuda/
  plugin.json
  rvc_plugin.py / rvc_engine.py / shared_audio.py
  transient_gate.py / spectral_gate.py
  runtime/python.exe
  infer/、configs/
  assets/hubert/hubert_base.pt
  assets/rmvpe/rmvpe.pt
  models/モデル.pth、検索.index
```

既存のRVC実行環境から配置する場合は、アプリのルートで次のスクリプトに参照元を指定します。実行前に引数と必要容量を確認してください。モデルや依存物を新たにダウンロードするスクリプトではありません。

```powershell
cd C:\develop\VoiceMixerMicRouter
pwsh -File .\dev\scripts\stage-rvc-cuda-plugin.ps1 -SourceRoot 'D:\RVC環境' -KahoRoot 'D:\利用許諾のあるモデル'
```

既存環境をそのまま使う場合は、停止中のmic-settings.jsonのplugin_directoryへ、そのrvc-cudaを含むpluginsフォルダーを指定します。

モデル・Indexはプラグイン基準の相対パス、または別フォルダーの絶対パスで指定できます。Index適用率0ではIndexを使いません。RVC設定画面の各項目にマウスを重ねると意味・範囲・再起動要否を表示します。

短いノイズの抑制は既存の推論入力履歴で衝撃音を判定し、対応する変換後区間を抑えます。追加の音声待ちを設けませんが、短い発声との誤判定や声と重なるクリックの限界があります。
