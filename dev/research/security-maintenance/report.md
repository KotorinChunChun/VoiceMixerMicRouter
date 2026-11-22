# 日次セキュリティ調査

## 2026-11-05 PyTorch公告の別名照合

音声分離プラグインのrequirements.txtにはtorch2.5.1/torchaudio2.5.1/demucs4.0.1が固定されている。[GHSA-63cw-57p8-fm3p](https://github.com/pytorch/pytorch/security/advisories/GHSA-63cw-57p8-fm3p)が本日のOSV照合で新規IDとして現れたが、CVE-2026-24747/PYSEC-2026-2286は前回から追跡済み。同じ問題の別名であり新しい実攻撃件数へ加算しない。

公式原文公開2026年1月26日（OSV登録1月27日）、今回OSV更新11月4日。影響<=2.9.1、修正>=2.10.0。細工されたcheckpointをtorch.load(..., weights_only=True)で読み込む場合にもメモリー破壊と任意実行につながる。weights_only=Trueだけで回避できるとは扱わない。2.5.1の固定版は影響範囲内。

engines.pyではDemucsのget_model(descriptor(method)["model_entry"], repo=folder)を使う。自作ソースに直接torch.loadの呼出しは検出しないが、Demucsの推移読込み経路があるため安全と断定しない。コードには選択methodや管理モデルの制約があるが、通常Python環境/実モデルの導入状態/実読込み/攻撃者制御の成立までは未確認。モデル・checkpoint・音声・個人設定を読まず、取得・実行・GPU/実機試験をしない。

[公式修正版配布](https://pypi.org/project/torch/2.10.0/)のmetadataと配布情報を確認した。torch/torchaudio/CUDA/DeepFilterNet/Demucsの世代・互換性評価を伴うため既存のB（ML依存移行設計待ち）を継続する。2.5.1→2.10.0を無条件置換せず、未回答を承認とせず新規の重複質問もしない。信頼できないcheckpointを読み込まないことが暫定回避策。

製品ソース/依存pin/通常環境/モデル/サービスは未変更、未配布。文書のみ差分・リンクを検査し、形式的な製品build/音声/GUI試験は実施しない。[横断の日次報告](C:/develop/default/dev/research/security-maintenance/2026-11-05.md)で既存保留と確認範囲を管理する。

## 2026-11-22 heaplessの9公告とWindows依存の照合

調査開始時のHEADは `7fe658683d29972da8397275a8ecbfb674ba528a`、GitHub mainは `b1ff6375dda1433d5ee422b889a7bd84e366f5ab`。両方のCargo.tomlとCargo.lockは一致した。既存のREADME.mdの編集と未追跡LICENSEを保持して調査した。

`cargo tree --locked --offline --manifest-path src/src-tauri/Cargo.toml --target x86_64-pc-windows-msvc -i heapless -e normal,build,features` で、通常構成に `voicemixer-mic-router → oxiaudio-encode 0.2.1 → flacenc 0.5.1 → heapless 0.8.0` が選択されることを確認した。heaplessはdefaultとserdeが有効。前回のQUIC未選択の判定は維持する。

### 対象APIの利用状況

公式レジストリから取得済みのflacencとoxiaudio-encode、製品ソースを調べた。flacencではheaplessのVecをFLACヘッダーのu8、予測用初期値のi32、LPC係数のf32/f64に使う。製品の共有Backendには `recording/writer.rs::write_flac` からFLACエンコーダーを使う経路がある。パッケージが選択されることと、各公告の発生条件を満たすことを次の表で照合した。

| 公告 | 対象と確認結果 |
|---|---|
| [0574](https://rustsec.org/advisories/RUSTSEC-2026-0574.html) | SortedLinkedListのFindMutView::pop。調べた依存経路に使用なし。 |
| [0575](https://rustsec.org/advisories/RUSTSEC-2026-0575.html) | Spsc::peek。調べた依存経路に使用なし。 |
| [0576](https://rustsec.org/advisories/RUSTSEC-2026-0576.html) | IndexMap::truncate。調べた依存経路に使用なし。 |
| [0577](https://rustsec.org/advisories/RUSTSEC-2026-0577.html) | Deque、HistoryBuf、IndexMapとVec::IntoIterの、要素のDropがpanicする場合の処理。Vec以外の型は使用なし。実経路のVec要素は数値型で、panicするDropを持たない。 |
| [0578](https://rustsec.org/advisories/RUSTSEC-2026-0578.html) | SortedLinkedList::zeroize。調べた依存経路に使用なし。 |
| [0579](https://rustsec.org/advisories/RUSTSEC-2026-0579.html) | Deque::try_from。公式の関数別範囲は0.9.2以上0.9.4未満で、現行0.8.0はこの関数別範囲外。 |
| [0580](https://rustsec.org/advisories/RUSTSEC-2026-0580.html) | arc_pool、box_pool、object_pool、旧pool。調べた依存経路に使用なし。 |
| [0581](https://rustsec.org/advisories/RUSTSEC-2026-0581.html) | Deque::make_contiguous。調べた依存経路に使用なし。 |
| [0582](https://rustsec.org/advisories/RUSTSEC-2026-0582.html) | IndexMap::insert。調べた依存経路に使用なし。 |

この確認範囲では公告の不具合が成立する呼出しを検出しなかった。修正版への依存更新は未完了で、lock上の一致は残る。

### 互換性の隔離検証と採用判断

2026-11-22にcrates.ioの公式metadataを取得し、最新安定版がflacenc 0.5.1、oxiaudio-encode 0.2.1、heapless 0.9.4であることを確認した。[上流のCargo.toml](https://github.com/yotarok/flacenc-rs/blob/main/Cargo.toml)もheaplessを `0.8.0` に指定している。この制約は0.9.4を許容しない。[heaplessの変更履歴](https://github.com/rust-embedded/heapless/blob/main/CHANGELOG.md)では、0.9でVec::from_slice等のエラー型が `()` からCapacityErrorに変わり、0.9.4の最小Rust版は1.88。製品の宣言は1.85である。

公式flacenc 0.5.1を同版ログ内へコピーし、そのコピーのheapless制約だけを `=0.9.4` に変更した。独立したprobeから `cargo check --target x86_64-pc-windows-msvc` を実行したところ、終了コード101となった。`component/datatype.rs` の2007行目と2100行目の `map_err(|()| ...)` でCapacityErrorとの型不一致E0308が発生した。製品の通常音声・接続・停止・全体ビルドへ進める候補には至っていない。

取得先はcrates.io公式レジストリ。heapless 0.9.4の配布crateのSHA-256は `411fdef4df93751117bd9ef8caa0e7f99c65dafec5bd84313eb8e2fdd75e32f2` で、隔離lockと一致した。依存ツリー、隔離manifest差分、コンパイル出力と開始時のハッシュは `dev/history/v1.0.32/logs/security-heapless-20261122/` に保存した。

今回の採用版はなし。公式の互換パッチとしてlockだけを変更できず、上流ライブラリの互換修正とRust最小版の検討が必要なため採用を保留する。通常Cargo.tomlとCargo.lockのSHA-256は開始時と一致し、復旧操作は不要。隔離試験を再開する場合は同フォルダーのコピーを使う。製品へ採用する前に、上流の修正版または互換修正案の検証、合成PCMのFLAC往復、通常音声・接続・停止と製品ビルドが必要である。

## 起動コマンド（案内のみ）

任意のフォルダーから設定画面を開く既存入口。本調査では実行していない。release/MicRouterの配備済みEXE・設定が必要で、不在時に自動インストールはしない。

pwsh -File "C:/develop/KotorinToys/tools/VoiceMixerMicRouter/dev/scripts/Start-MicRouter.ps1" -Mode Settings
