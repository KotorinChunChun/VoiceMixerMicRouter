# 同梱flacencの変更

crates.io公式配布のflacenc 0.5.1から、src、build.rs、Cargo.toml、LICENSE、README.mdを収録した。著作権とApache-2.0ライセンスはLICENSEに記載する。

- heaplessを0.8.0から0.9.4へ変更した。
- component/datatype.rsの2か所で、Vec::from_sliceのエラーをCapacityErrorとして受け取れるようにした。VerifyErrorの内容と失敗条件は従来どおり。
- source.rsのFrameBuf::with_sizeは、1～31フレームの最終ブロック用に最小容量32のバッファを作る。実際に格納したサンプル数はfilled_sizeで保持する。RFC 9639の4.1節と8.2節で認められる短い最終フレームの保存失敗を防ぐ。

公式配布元は https://crates.io/crates/flacenc/0.5.1 、上流リポジトリは https://github.com/yotarok/flacenc-rs/ 。
