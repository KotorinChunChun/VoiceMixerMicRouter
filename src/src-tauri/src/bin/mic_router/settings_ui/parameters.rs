//! RVCパラメーターの表示形式と型・範囲の検証。
use audio_mixer_backend::plugins::manifest::{ParameterDescriptor, ParameterKind};
use serde_json::Value;

pub(super) fn parameter_label(p: &ParameterDescriptor) -> String {
    format!(
        "{}{}{}",
        p.label,
        if p.restart_required { " ＊" } else { "" },
        if p.unit.is_empty() {
            String::new()
        } else {
            format!("（{}）", p.unit)
        }
    )
}

pub(super) fn parameter_text(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
}

pub(super) fn parameter_value_changed(value: &Value, previous: &Value) -> bool {
    value != previous
        && !(value.is_number() && previous.is_number() && value.as_f64() == previous.as_f64())
}

pub(super) fn parse_parameter(
    p: &ParameterDescriptor,
    text: &str,
    on: bool,
) -> Result<Value, String> {
    match p.kind {
        ParameterKind::Boolean => Ok(Value::Bool(on)),
        ParameterKind::Number | ParameterKind::Integer => {
            let value = text
                .trim()
                .parse::<f64>()
                .map_err(|_| format!("{}には数値を入力してください", p.label))?;
            if !value.is_finite()
                || p.min.is_some_and(|min| value < min)
                || p.max.is_some_and(|max| value > max)
                || (p.kind == ParameterKind::Integer && value.fract() != 0.0)
            {
                return Err(format!(
                    "{}の範囲は {} ～ {}{} です",
                    p.label,
                    p.min.map_or("制限なし".into(), |v| v.to_string()),
                    p.max.map_or("制限なし".into(), |v| v.to_string()),
                    if p.kind == ParameterKind::Integer {
                        "（整数）"
                    } else {
                        ""
                    }
                ));
            }
            Ok(if p.kind == ParameterKind::Integer {
                Value::from(value as i64)
            } else {
                Value::from(value)
            })
        }
        ParameterKind::Enum if !p.options.iter().any(|v| v == text) => {
            Err(format!("{}を選択してください", p.label))
        }
        ParameterKind::File => Ok(Value::String(text.trim().trim_matches('"').to_owned())),
        _ => Ok(Value::String(text.to_owned())),
    }
}

pub(super) fn parameter_help(p: &ParameterDescriptor) -> String {
    let description = match p.id.as_str() {
        "model" => "変換先の声を決めるRVCモデル（.pth）です。候補以外も、任意のフォルダーのファイルをフルパスで直接入力・貼り付けできます。相対パスはrvc-cudaフォルダー基準です。右のボタンで入力中のファイルの保存フォルダーを開きます。",
        "index" => "モデルに対応する特徴検索用ファイル（.index）です。任意のフルパスを入力できます。Index適用率が0のときは使用しません。右のボタンで入力中のファイルの保存フォルダーを開きます。",
        "pitch" => "声の高さを半音単位で変えます。正の値で高く、負の値で低くします。12半音で1オクターブです。",
        "formant" => "声の響き・声道の特徴を調整します。ピッチとは別に声質を変え、0で補正しません。",
        "f0_method" => "声の基本周波数（ピッチ）を抽出する方式です。処理負荷と精度が異なり、RMVPE等は対応する追加モデルが必要です。",
        "index_rate" => "検索Indexの特徴を混ぜる割合です。0でIndexを使用せず、値を上げるほどIndexの影響を強めます。0より大きい値にはモデルと対応するIndexファイルが必要です。",
        "block_time" => "1回の推論で処理する音声の長さです。長くすると処理は安定しやすい一方、遅延が増えます。短すぎると処理が追いつかず音切れする場合があります。",
        "crossfade_time" => "推論ブロックの境目を重ねて滑らかにつなぐ時間です。SOLAによる接続に使い、境目のノイズと処理時間に影響します。",
        "extra_time" => "推論のために前の音声から追加で参照する履歴の長さです。長いほど処理負荷・メモリ使用量が増えます。この値そのものが音声の待ち時間になるわけではありません。",
        "rms_mix_rate" => "入力と変換後の音量変化を混ぜる割合です。0は入力の音量変化、1は変換後の音量変化を基準にし、中間は両者を混合します。",
        "threshold_db" => "小さな入力音を無音にするゲートのしきい値です。0dBに近づけるほど小さい声も切れやすくなります。-60dBで入力ゲートを無効にします。",
        "n_cpu" => "CPUで行うピッチ抽出のワーカー数の上限です。方式によって効果が異なり、増やすとCPU使用量が増えます。",
        "use_jit" => "対応するRVCのJIT推論経路を使います。モデル・GPU・実行環境によって対応状況が異なります。",
        "input_noise_reduction" => "RVC推論前の音へSpectral Gateを適用して背景ノイズを抑えます。基本設定の短い衝撃音の抑制とは別の処理で、声にも影響する場合があります。",
        "output_noise_reduction" => "RVC変換後の音へSpectral Gateを適用して背景ノイズを抑えます。基本設定の短い衝撃音の抑制とは別の処理で、声にも影響する場合があります。",
        _ => &p.description,
    };
    let mut result = description.to_owned();
    if let (Some(min), Some(max)) = (p.min, p.max) {
        result.push_str(&format!("\n範囲：{min}～{max}{}。", p.unit));
    }
    result.push_str(if p.restart_required {
        "\n適用するとRVCを再起動します。"
    } else {
        "\n適用すると起動中のRVCにも反映します。"
    });
    result
}
