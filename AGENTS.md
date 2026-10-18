# VoiceMixerMicRouter の作業ルールと資料目録

VoiceMixerから分離した独立Gitプロジェクト。ソースの正本はこのリポジトリで管理する。

- 開始時にgit statusを確認し利用者の変更を保持する。質問には回答のみとする。
- 文書・コミットは日本語、PowerShellは必ずpwshを使う。
- 版番号は末尾だけ1ずつ増やす。上位桁は明示依頼時のみ変更し、Cargoと画面・CLIを揃える。
- 仕様変更時は実装・回帰・仕様書を揃える。多項目は工程別に検証し日本語コミットして進む。
- dev直下を進行中のreq/imp/task、dev/history/vX.Y.Zを完成資料、dev/testingを継続記録とする。古い未チェック欄より後続の完了記録を優先する。
- dev/privateとdev/tempはGit・公開・通常の調査対象から除外する。個人設定、モデル、実行環境、録音、認証情報を追跡しない。
- GUI変更では対象ウィンドウだけを検証・撮影し、保存済み設定と未保存編集、元の起動状態を保持する。音声の停止前には案内する。
- 共存の入力透過、上書きの主キー対、MSTSC検知、RVC背景準備、MIC復帰、P2Tの一時排他と解放を維持する。
- 完了報告には任意のフォルダーから使える起動コマンドを必ず載せる。
- ローカルコミット、外部公開、署名、インストールを別工程として依頼範囲で実施する。

| 目的 | 資料 |
|---|---|
| 利用・概要 | README.md、docs/mic-resident.md |
| 現行仕様 | docs/mic-resident-spec.md |
| ビルド・開発 | docs/DEVELOPERS_GUIDE.md |
| 内部構造・配備 | docs/architecture/mic-resident.md |
| 進行中の依頼 | dev直下の対応するimp/req/task |
| 検証・過去の判断 | dev/testing、dev/historyの必要な版 |
| 公開 | dev/RELEASE_RULE.md |

src/src-tauriの名前は分離時の相対パス互換のため維持する。TauriのWeb画面は含まずWin32常駐だけをビルドする。共通音声Backendはこのリポジトリ内にソースのスナップショットとして保持し、元リポジトリ・ローカル絶対パス・非公開Git依存をビルド条件にしない。
