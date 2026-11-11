# 開発者ガイド

## 構成

アプリのコードはsrc/src-tauri/src/bin/voicemixer-mic.rsとmic_router配下にあります。src/src-tauri/srcの他のRustモジュールは同梱の音声Backendです。src/src-tauri/resourcesとplugins/voice-extractionはBackendに埋め込む素材です。plugins/rvc-cudaにはRVCのプロトコル、Python処理、設定スキーマがあります。モデルやPython環境はGit対象外です。

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

初回配備では実デバイスから設定を生成します。RVCを使うにはplugins/rvc-cuda/README.mdに記載の環境を別途用意します。既存の実行環境を使う場合は、そのpluginsフォルダーをplugin_directoryに指定します。モデル／Indexは絶対パスでも指定できます。

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

音声試験は環境条件を確認してから、対象の試験だけignoreを解除します。--ignoredだけを指定して全試験を実行しないでください。モデル、OSの既定設定の変更、録音などの追加条件は各テストに記載しています。

## 常駐と分離したCLI試験

Windowsでは `--test-scenario FILE` で、JSONに記述した操作を順番に実行できます。通常の常駐を起動したまま実行できます。試験ごとに初期値から始まり、常駐ウィンドウや管理パイプ、利用者の設定、実デバイスに接続しません。操作結果は標準出力にUTF-8のJSONで1件返します。

```powershell
$report = & .\src\src-tauri\target\debug\voicemixer-mic.exe --test-scenario `
    .\dev\scripts\tests\fixtures\isolated-cli.json | ConvertFrom-Json
$report.steps | Select-Object step, op, ok
```

入力は1 MiB以下のUTF-8 JSONです。`schema_version` に1、`steps` に1〜256件の操作を指定します。[シナリオ例](../dev/scripts/tests/fixtures/isolated-cli.json)は設定適用、RVC無効時の起動、出力3系統の切替と同時押下、効果音、オーバーレイ、4タブのリセットを検証します。

| 操作 | 入力と確認できる内容 |
|---|---|
| `inspect` | `detail.applied` と `detail.draft` に適用済みと編集中の設定を出す |
| `settings_patch` | `values` に変更する設定キーと値を指定する。デバイスは仮のIDを使用できる。 `rvc_parameters` は指定したキーだけを更新する |
| `settings_key` | `row` に `inspect` の `hotkeys` の保存順を1〜14で指定し、`binding` に1行分のキー設定を指定する |
| `settings_reset` | `tab` に `basic`、`keys`、`rvc`、`sounds` のいずれかを指定し、該当タブの下書きを初期値に戻す |
| `settings_apply` / `restart` | 下書きの検証と適用、または適用済み設定のJSONからの復元と起動モード判定。再起動時に下書きと押下状態を解放する |
| `mode` / `toggle_mode` / `release_rvc` / `toggle_noise` | モード切替、RVC解放、短いノイズ抑制の設定。 `mode` の値は `direct`、`rvc`、`speaker_rvc`、`muted` |
| `output_toggle` / `output_hold` | `output` に1〜3、押下操作では `pressed` に真偽値を指定する。通常・押下・実効出力をそれぞれ `state.outputs` で確認できる |
| `sound_preview` | `choice` に0〜7を指定する。効果音の波形を生成し、フレーム数、ピーク、試聴の出力先を報告する |
| `overlay_cycle` / `overlay_show` / `overlay_elapse` | 表示モード切替、再表示、経過時間の模擬。経過時間は `milliseconds` で進め、表示状態と残り秒数を確認する |

各操作は `ok` と操作後の `state` を返します。操作が失敗した場合は `error` に理由を出し、後続の操作も実行します。終了コードは、すべての操作が成功した場合は0、失敗した操作がある場合は1、入力JSONや引数などに問題があり試験を開始できなかった場合は2です。`--config` や `--stop` などの通常の引数とは併用できません。

設定の検証、音声経路の生成、効果音の波形、オーバーレイの表示条件を確認します。Windowsのキー入力・実画面、音声の入出力と試聴、実デバイスの接続やRVCモデルの推論は、対応するGUI試験や実機診断で確認します。

配布候補EXEのJSON出力と終了コードを検査するには、`Test-IsolatedCli.ps1` に対象EXEを指定します。 `-ProtectedSettings` を指定すると、試験前後でそのファイルのハッシュも照合します。

```powershell
.\dev\scripts\tests\Test-IsolatedCli.ps1 -Executable .\src\src-tauri\target\debug\voicemixer-mic.exe
```
## 公開インストーラ（v1.0.18）

.NET 10 SDKと上記Rust環境で実行します。実行時の.NET追加導入は不要です。Inno Setupは署名・固定SHA256検証後、artifacts内へローカル配置します。

```powershell
cd C:\develop\VoiceMixerMicRouter
pwsh -File .\dev\scripts\Build-Installer.ps1 -RequireClean
```

配布EXEは静的CRT、helperは自己完結型です。artifacts/installersの実行別フォルダーへsetup、製品別台帳、SHA256、ソースコミット／ファイル一覧を生成します。公開するのはsetup・台帳・SHA256と導入／登録案内です。公開ビルドに-SkipBuildを使わず、クリーンな採用ソースから作り直してください。

通常の常駐を終了してから、生成したsetupをTest-Installer.ps1の-Installerに指定します。隔離先で新規導入、設定編集中の更新拒否、同じバージョンでの更新、削除、保持を確認できます。すでに正式導入したPCでは試験を拒否します。別セッション、実ログイン、KotorinToys本体の画面は管理PCで受入を確認します。
