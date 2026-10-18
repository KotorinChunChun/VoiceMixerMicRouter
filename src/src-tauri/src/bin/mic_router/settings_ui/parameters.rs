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
        _ => Ok(Value::String(text.to_owned())),
    }
}
