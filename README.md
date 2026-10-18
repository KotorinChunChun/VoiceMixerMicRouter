# VoiceMixerMicRouter

Windowsのタスクトレイに常駐し、MIC・MIC-RVC・SPK-RVC・停止を切り替えて、3つの出力デバイスへ音声を送ります。

VoiceMixer本体から独立したWin32アプリです。RVCモデルは初回選択時にバックグラウンドで準備し、読込中もMICへ戻せます。各出力のON/OFF、押している間ON、短い衝撃音の抑制、移動可能なオーバーレイに対応します。

- [利用方法](docs/mic-resident.md)
- [現行仕様](docs/mic-resident-spec.md)
- [ビルド・開発](docs/DEVELOPERS_GUIDE.md)
- [内部構造](docs/architecture/mic-resident.md)

## ビルド

Windows、Rust MSVCツールチェーン、Visual StudioのC++ビルドツール／Windows SDKが必要です。

```powershell
cd C:\develop\VoiceMixerMicRouter
cargo build --manifest-path .\src\src-tauri\Cargo.toml --release --bin voicemixer-mic
pwsh -File .\dev\scripts\Install-MicRouter.ps1 -SkipBuild -NoStartup -NoStart
pwsh -File .\dev\scripts\Start-MicRouter.ps1 -Mode Settings
```

初回設定には有効な録音・再生デバイスが必要です。RVCには別途、利用許諾のあるモデルとCUDA対応のPython実行環境が必要です。モデル・CUDA／Pythonランタイム・個人設定はこのリポジトリに含みません。初回配備スクリプトは設定がない場合だけ既定設定を生成します。既存の環境を使う場合はmic-settings.jsonのplugin_directoryに配置済みpluginsフォルダーを指定できます。

## ソースの由来

2026-10-19にVoiceMixerの常駐v1.0.16から分離しました。共通音声Backendは外部の非公開リポジトリへ依存せず、本リポジトリ内で管理します。src/src-tauriという名前は既存の相対参照を維持するため残していますが、Tauri本体やWeb画面はビルドしません。不要な本体の実行バイナリ・フロントエンド・元のGit履歴は含みません。

RVC関連の上流ライセンスは[プラグイン内](plugins/rvc-cuda/RVC_LICENSE.txt)に保持します。モデルの入手・利用条件は各配布元で確認してください。
