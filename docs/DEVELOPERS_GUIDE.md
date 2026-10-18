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

## 公開インストーラ（v1.0.18）

.NET 10 SDKと上記Rust環境で実行します。実行時の.NET追加導入は不要です。Inno Setupは署名・固定SHA256検証後、artifacts内へローカル配置します。

```powershell
cd C:\develop\VoiceMixerMicRouter
pwsh -File .\dev\scripts\Build-Installer.ps1 -RequireClean
```

配布EXEは静的CRT、helperは自己完結型です。artifacts/installersの実行別フォルダーへsetup、製品別台帳、SHA256、ソースコミット／ファイル一覧を生成します。公開するのはsetup・台帳・SHA256と導入／登録案内です。公開ビルドに-SkipBuildを使わず、クリーンな採用ソースから作り直してください。

通常常駐を終了してから、生成したsetupをTest-Installer.ps1の-Installerへ渡すと、隔離先で新規導入・設定編集中の更新拒否・同版の更新・削除・保持を確認します。既存の正式導入があるPCでは試験を拒否します。別セッション・実ログイン・KotorinToys本体UIは管理PCの受入対象です。
