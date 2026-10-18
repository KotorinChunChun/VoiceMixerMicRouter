# KotorinToysへの登録引き継ぎ

登録作業は管理PC側で行います。本リリースでは本体のソース・config・登録・Releasesを変更していません。

## 製品情報

| 項目 | 値 |
|---|---|
| id | voice-mixer-mic-router |
| name | VoiceMixerMicRouter |
| appId | KotorinToys.VoiceMixerMicRouter |
| version | 1.0.18 |
| installedExecutable | VoiceMixerMicRouter.exe |
| repository | KotorinChunChun/VoiceMixerMicRouter |
| tag | v1.0.18 |
| installer | VoiceMixerMicRouter-1.0.18-setup.exe |
| 台帳 | VoiceMixerMicRouter-packages.json |

Releaseの台帳のSHA-256と実ファイルを照合し、本体の固定ProductIdentityへ製品を追加します。mainのconfig/tools.jsonへ[自己完結した追加項目](kotorintoys-tool-entry.json)を追加します。開発登録はtool.jsonと独立Gitの採用コミットを使用します。configだけの追加では現在の本体の製品ホワイトリストを通過しないため、管理PCで本体をビルド・更新してください。

ProductIdentityの追加値：new("voice-mixer-mic-router", "VoiceMixerMicRouter", "KotorinToys.VoiceMixerMicRouter")。固定配布先は製品名と同じVoiceMixerMicRouterです。

本体の配布スクリプトは現在C#製品用の固定表を持つため、MicRouterの作成には独立側のdev/scripts/Build-Installer.ps1を使用してください。製品名だけを既存C#の発行対象へ追加してもRustはビルドされません。

## 管理契約

パイプはKotorinToys.<現在ユーザーSID>.voice-mixer-mic-router。同一ユーザー所有、同一Windowsセッション、リモート接続拒否。UTF-8改行区切りのstatusまたはJSON要求を受けます。応答はprotocolVersion=2、pid、ready、version、launchOrigin、capabilities、requestId、result、detail。管理応答へ音声・デバイスの個人設定は含めません。

show/stopはexpectedPidを必須とし、不一致はrefused。showは設定画面を表示してcompleted。stopは設定表示中・適用中ならrefused、可能ならacceptedを返した後に正常終了します。acceptedを停止完了とは扱わず本体で実プロセスの終了を待ってください。未対応のshortcutsはunsupportedです。

起動は--startup --launch-origin host_manual|direct_task|via_kotorintoys。startupでは設定画面とオーバーレイを最初は非表示にし、既存設定の音声モードを復元します。初回設定がなければ送信停止で作成します。通常の単独初回起動は設定画面を開きます。

## 配布・保守

利用者単位のInno Setup、HKCUのKotorinToys.VoiceMixerMicRouter_is1、導入先のtool.jsonとEXEのProductName/FileVersionを照合できます。未署名の開発版としてunsigned-public-buildを台帳へ記録します。

保守helperは本製品専用です。更新時は製品・導入先・正常停止を確認し、削除時は本体に対象の所有記録がある予約だけを既存の照合・復旧方式で整理します。未登録PCに本体設定を作成せず、外部のStartup登録やモデルを削除しません。

RVCモデル・実行環境と仮想オーディオドライバーは任意の外部依存です。設定はLocalAppData/VoiceMixerMicRouterへ保存し、更新と削除で保持します。導入手順は[インストール案内](installation.md)を参照してください。

## 確認範囲

独立版の通常回帰と.NET CurrentUserOnlyの実接続・GUI・インストーラの検証を行います。管理PCの本体登録・本体の実画面での導入操作・実ログインは本リポジトリでの実測に含めず、管理PC側の受入とします。
