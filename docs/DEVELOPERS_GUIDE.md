# 開発者ガイド

## 構成

src/src-tauri/src/bin/voicemixer-mic.rsとmic_router配下がアプリです。src/src-tauri/srcの他のRustモジュールは同梱音声Backendで、src/src-tauri/resourcesとplugins/voice-extractionはBackendの埋め込み素材です。plugins/rvc-cudaはRVCのプロトコル・Python処理・設定スキーマを保持します。モデルやPython環境はGit対象外です。

元のVoiceMixerチェックアウトや非公開Gitへの依存はありません。Cargo.lockを追跡します。旧Tauri用cfgはBackendのソース互換のため残っていますが、対応featureは公開せず、常駐単独のビルドから除外します。

## 確認・起動

```powershell
cd C:\develop\VoiceMixerMicRouter
cargo test --manifest-path .\src\src-tauri\Cargo.toml --bin voicemixer-mic
cargo check --manifest-path .\src\src-tauri\Cargo.toml --bin voicemixer-mic --all-features
cargo build --manifest-path .\src\src-tauri\Cargo.toml --release --bin voicemixer-mic
pwsh -File .\dev\scripts\Install-MicRouter.ps1 -SkipBuild -NoStartup -NoStart
pwsh -File .\dev\scripts\Start-MicRouter.ps1 -Mode Settings
```

初回配備は実デバイスから設定を生成します。RVCを使うにはplugins/rvc-cuda/README.mdの環境を別途用意します。既存の実行環境を使う場合はplugin_directoryをそのpluginsフォルダーへ設定します。モデル／Indexは絶対パスにも対応します。

デバッグ起動は次のとおりです。設定を本番と共用する場合、同じWindowsセッションの常駐が停止していることを先に確認します。

```powershell
cd C:\develop\VoiceMixerMicRouter
cargo run --manifest-path .\src\src-tauri\Cargo.toml --bin voicemixer-mic -- --config .\release\MicRouter\mic-settings.json --settings
```

GUI試験は利用中の常駐を終了してから、通常ユーザーのデスクトップで1件ずつ実行します。テストのために音声が停止することとフォーカス操作を先に案内します。

```powershell
cargo test --manifest-path .\src\src-tauri\Cargo.toml --bin voicemixer-mic settings_controls_roundtrip_and_page_layout -- --ignored --test-threads=1
cargo test --manifest-path .\src\src-tauri\Cargo.toml --bin voicemixer-mic native_commands_menu_icons_and_nonactivating_overlay -- --ignored --test-threads=1
```

音声試験のignoreは環境条件を確認して対象だけ解除します。--ignoredのみで全試験を走らせないでください。モデル、OS既定変更、録音等の追加条件は各テストに記載します。
