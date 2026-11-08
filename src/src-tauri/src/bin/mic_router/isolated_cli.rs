//! 操作を同一プロセス内で検証するCLI。常駐IPC・設定ファイル・実デバイスを使用しない。
use crate::{
    config::{
        default_hotkeys, initial_rvc_parameters, Hotkey, Mode, Settings, APP_VERSION,
        NOISE_PARAMETER,
    },
    engine::validate_speaker_route_with,
    settings_ui::parameters::{parameter_text, parse_parameter},
    sounds,
};
use audio_mixer_backend::plugins::{
    manifest::{ParameterDescriptor, ParameterKind},
    package::PluginPackageManifest,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::{
    ffi::OsString,
    fs::File,
    io::{Read, Write},
    path::Path,
    time::Duration,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    schema_version: u8,
    steps: Vec<Operation>,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Operation {
    Inspect,
    SettingsPatch { values: Map<String, Value> },
    SettingsKey { row: usize, binding: Hotkey },
    SettingsReset { tab: Tab },
    SettingsApply,
    Restart,
    Mode { mode: Mode },
    ToggleMode,
    ReleaseRvc,
    ToggleNoise,
    OutputToggle { output: u8 },
    OutputHold { output: u8, pressed: bool },
    SoundPreview { choice: u8 },
    OverlayCycle,
    OverlayShow,
    OverlayElapse { milliseconds: u64 },
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Tab {
    Basic,
    Keys,
    Rvc,
    Sounds,
}

struct State {
    applied: Settings,
    draft: Settings,
    mode: Mode,
    held: [bool; 3],
    overlay_elapsed: Duration,
    schema: Vec<ParameterDescriptor>,
}

impl State {
    fn new(directory: &Path) -> Result<Self, String> {
        let schema = serde_json::from_str::<PluginPackageManifest>(include_str!(
            "../../../../../plugins/rvc-cuda/plugin.json"
        ))
        .map_err(|error| error.to_string())?
        .descriptor
        .parameters;
        let mut applied = Settings::defaults(directory.to_path_buf());
        applied.reset_basic();
        applied.last_mode = Mode::Muted;
        Self::check_route(&applied, Mode::Muted, [false; 3])?;
        Ok(Self {
            draft: applied.clone(),
            applied,
            mode: Mode::Muted,
            held: [false; 3],
            overlay_elapsed: Duration::ZERO,
            schema,
        })
    }

    fn check_route(settings: &Settings, mode: Mode, held: [bool; 3]) -> Result<(), String> {
        settings.validate()?;
        let effective = settings.with_output_holds(held);
        // 隔離内では指定された論理IDを使う。OSの既定デバイス解決は行わない。
        validate_speaker_route_with(settings, mode, |id| Ok(id.to_owned()))?;
        validate_speaker_route_with(&effective, mode, |id| Ok(id.to_owned()))?;
        effective.project(mode)?;
        Ok(())
    }

    fn commit(
        &mut self,
        mut settings: Settings,
        mode: Mode,
        held: [bool; 3],
    ) -> Result<(), String> {
        let held = settings.device_selection().filter_holds(held);
        Self::check_route(&settings, mode, held)?;
        settings.last_mode = mode;
        self.applied = settings;
        self.draft.last_mode = mode;
        self.mode = mode;
        self.held = held;
        self.overlay_elapsed = Duration::ZERO;
        Ok(())
    }

    fn patch(&mut self, values: Map<String, Value>) -> Result<(), String> {
        let mut draft = serde_json::to_value(&self.draft).map_err(|error| error.to_string())?;
        for (key, value) in values {
            match key.as_str() {
                "rvc_parameters" => {
                    let parameters = value
                        .as_object()
                        .ok_or("rvc_parametersにはオブジェクトを指定してください")?;
                    let destination = draft[&key].as_object_mut().ok_or("RVC設定が不正です")?;
                    for (id, value) in parameters {
                        if !self.schema.iter().any(|p| p.id == *id) {
                            return Err(format!("未対応のRVC設定項目: {id}"));
                        }
                        destination.insert(id.clone(), value.clone());
                    }
                }
                "overlay_mode"
                | "startup_mode"
                | "rvc_enabled"
                | "output_labels"
                | "microphone_id"
                | "microphone_name"
                | "speaker_id"
                | "speaker_name"
                | "cable_id"
                | "cable_name"
                | "cable_enabled"
                | "voice_input_id"
                | "voice_input_name"
                | "voice_input_enabled"
                | "monitor_id"
                | "monitor_name"
                | "monitoring"
                | "hotkeys"
                | "switch_sounds"
                | "sound_device_id"
                | "sound_device_name" => draft[&key] = value,
                _ => return Err(format!("変更できない設定項目: {key}")),
            }
        }
        let mut next: Settings =
            serde_json::from_value(draft).map_err(|error| error.to_string())?;
        next.hotkey.clone_from(&self.draft.hotkey);
        self.draft = next;
        Ok(())
    }

    fn reset(&mut self, tab: Tab) {
        match tab {
            Tab::Basic => self.draft.reset_basic(),
            Tab::Keys => self.draft.hotkeys = default_hotkeys(),
            Tab::Rvc => {
                let defaults = initial_rvc_parameters();
                for p in self.schema.iter().filter(|p| p.id != NOISE_PARAMETER) {
                    self.draft.rvc_parameters[&p.id] =
                        defaults.get(&p.id).unwrap_or(&p.default).clone();
                }
            }
            Tab::Sounds => self.draft.reset_sounds(),
        }
    }

    fn apply(&mut self) -> Result<(), String> {
        let mut candidate = self.draft.clone();
        for label in &mut candidate.output_labels {
            *label = label.trim().to_owned();
        }
        let selected = candidate.device_selection().outputs;
        candidate.cable_enabled &= selected[0];
        candidate.voice_input_enabled &= selected[1];
        candidate.monitoring &= selected[2];
        for p in &self.schema {
            let Some(value) = candidate.rvc_parameters.get(&p.id) else {
                continue;
            };
            let right_type = match p.kind {
                ParameterKind::Boolean => value.is_boolean(),
                ParameterKind::Number | ParameterKind::Integer => value.is_number(),
                _ => value.is_string(),
            };
            if !right_type {
                return Err(format!("RVC設定の値の型が不正です: {}", p.id));
            }
            let parsed =
                parse_parameter(p, &parameter_text(value), value.as_bool().unwrap_or(false))?;
            candidate.rvc_parameters[&p.id] = parsed;
        }
        let mode = candidate.available_mode(self.mode);
        self.commit(candidate, mode, self.held)?;
        self.draft = self.applied.clone();
        Ok(())
    }

    fn switch_mode(&mut self, mode: Mode, play_sound: bool) -> Result<Value, String> {
        if mode.is_rvc() && !self.applied.rvc_enabled {
            return Err("RVCが無効です".into());
        }
        if !self.applied.device_selection().has_source(mode) {
            return Err("指定モードの入力デバイスが未選択です".into());
        }
        let changed = self.mode != mode;
        self.commit(self.applied.clone(), mode, self.held)?;
        Ok(if changed && play_sound {
            self.sound_result(self.applied.switch_sounds[mode.sound_index()])
        } else {
            Value::Null
        })
    }

    fn sound_result(&self, choice: u8) -> Value {
        let samples = sounds::samples(choice);
        json!({
            "choice": choice, "label": sounds::LABELS[choice as usize],
            "device_id": self.applied.sound_device_id,
            "frames": samples.len() / 2, "channels": 2, "sample_rate_hz": 48000,
            "peak": samples.into_iter().map(f32::abs).fold(0.0_f32, f32::max),
            "played": false
        })
    }

    fn execute(&mut self, operation: Operation) -> Result<Value, String> {
        match operation {
            Operation::Inspect => return Ok(json!({"applied": self.applied, "draft": self.draft})),
            Operation::SettingsPatch { values } => self.patch(values)?,
            Operation::SettingsKey { row, binding } => {
                let target = row
                    .checked_sub(1)
                    .and_then(|index| self.draft.hotkeys.get_mut(index))
                    .ok_or("キー設定の番号は1〜14で指定してください")?;
                *target = binding;
            }
            Operation::SettingsReset { tab } => self.reset(tab),
            Operation::SettingsApply => self.apply()?,
            Operation::Restart => {
                let saved =
                    serde_json::to_value(&self.applied).map_err(|error| error.to_string())?;
                let mut restored: Settings =
                    serde_json::from_value(saved).map_err(|error| error.to_string())?;
                restored.hotkey = restored.hotkeys[2].label();
                let mode = restored.effective_startup_mode(None);
                self.commit(restored, mode, [false; 3])?;
                self.draft = self.applied.clone();
            }
            Operation::Mode { mode } => return self.switch_mode(mode, true),
            Operation::ToggleMode | Operation::ReleaseRvc => {
                if !self.applied.rvc_enabled {
                    return Err("RVCが無効です".into());
                }
                let release = matches!(operation, Operation::ReleaseRvc);
                let mode = if release {
                    Mode::Direct
                } else {
                    self.mode.other()
                };
                return self.switch_mode(mode, !release);
            }
            Operation::ToggleNoise => {
                if !self.applied.rvc_enabled {
                    return Err("RVCが無効です".into());
                }
                let mut candidate = self.applied.clone();
                let enabled = !candidate.noise_suppression();
                candidate.set_noise_suppression(enabled);
                self.commit(candidate, self.mode, self.held)?;
                self.draft.set_noise_suppression(enabled);
            }
            Operation::OutputToggle { output } => {
                let index = output_index(output)?;
                let mut candidate = self.applied.clone();
                candidate.toggle_output(index);
                self.commit(candidate, self.mode, self.held)?;
                match index {
                    0 => self.draft.cable_enabled = self.applied.cable_enabled,
                    1 => self.draft.voice_input_enabled = self.applied.voice_input_enabled,
                    _ => self.draft.monitoring = self.applied.monitoring,
                }
                return Ok(self.sound_result(self.applied.switch_sounds[4 + index]));
            }
            Operation::OutputHold { output, pressed } => {
                let index = output_index(output)?;
                if pressed && !self.applied.device_selection().outputs[index] {
                    return Err("押下対象の出力デバイスが未選択です".into());
                }
                if self.held[index] != pressed {
                    let mut held = self.held;
                    held[index] = pressed;
                    self.commit(self.applied.clone(), self.mode, held)?;
                    return Ok(self.sound_result(self.applied.switch_sounds[4 + index]));
                }
            }
            Operation::SoundPreview { choice } => {
                if sounds::LABELS.get(choice as usize).is_none() {
                    return Err("効果音は0〜7で指定してください".into());
                }
                let mut sound = self.sound_result(choice);
                // 効果音タブの試聴は「適用」前でも編集中のデバイスを使う。
                sound["device_id"] = json!(self.draft.sound_device_id);
                return Ok(sound);
            }
            Operation::OverlayCycle => {
                let next = self.applied.overlay_mode.next();
                self.applied.overlay_mode = next;
                self.draft.overlay_mode = next;
                self.overlay_elapsed = Duration::ZERO;
            }
            Operation::OverlayShow => self.overlay_elapsed = Duration::ZERO,
            Operation::OverlayElapse { milliseconds } => {
                self.overlay_elapsed = self
                    .overlay_elapsed
                    .saturating_add(Duration::from_millis(milliseconds));
            }
        }
        Ok(Value::Null)
    }

    fn snapshot(&self) -> Value {
        let overlay = self.applied.overlay_mode;
        json!({
            "mode": self.mode, "rvc_enabled": self.applied.rvc_enabled,
            "noise_suppression": self.applied.noise_suppression(), "startup_mode": self.applied.startup_mode,
            "draft_changed": self.draft != self.applied,
            "outputs": {"saved": self.applied.output_flags(), "held": self.held,
                "effective": self.applied.with_output_holds(self.held).output_flags()},
            "overlay": {"mode": overlay, "visible": overlay.visible_after(self.overlay_elapsed),
                "remaining_seconds": overlay.brief_remaining(self.overlay_elapsed)}
        })
    }
}

fn output_index(output: u8) -> Result<usize, String> {
    if (1..=3).contains(&output) {
        Ok(usize::from(output - 1))
    } else {
        Err("出力番号は1〜3で指定してください".into())
    }
}

fn evaluate(path: &Path) -> Result<(bool, Vec<Value>), String> {
    let path = path.canonicalize().map_err(|error| error.to_string())?;
    let mut bytes = Vec::new();
    let mut file = File::open(&path).map_err(|error| error.to_string())?;
    if !file
        .metadata()
        .map_err(|error| error.to_string())?
        .is_file()
    {
        return Err("シナリオには通常ファイルを指定してください".into());
    }
    const MAX_BYTES: u64 = 1024 * 1024;
    (&mut file)
        .take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("シナリオは1 MiB以下にしてください".into());
    }
    let scenario: Scenario =
        serde_json::from_slice(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(&bytes))
            .map_err(|error| error.to_string())?;
    if scenario.schema_version != 1 || !(1..=256).contains(&scenario.steps.len()) {
        return Err("schema_versionは1、stepsは1〜256件を指定してください".into());
    }
    let mut state = State::new(path.parent().ok_or("シナリオの配置先がありません")?)?;
    let mut ok = true;
    let mut results = Vec::with_capacity(scenario.steps.len());
    for (index, operation) in scenario.steps.into_iter().enumerate() {
        let request = serde_json::to_value(&operation).map_err(|error| error.to_string())?;
        let name = &request["op"];
        let result = state.execute(operation);
        ok &= result.is_ok();
        results.push(match result {
            Ok(detail) => json!({"step": index + 1, "op": name, "ok": true, "detail": detail, "state": state.snapshot()}),
            Err(error) => json!({"step": index + 1, "op": name, "ok": false, "error": error, "state": state.snapshot()}),
        });
    }
    Ok((ok, results))
}

pub fn run(args: &[OsString]) -> i32 {
    let result = match args {
        [flag, path] if flag == "--test-scenario" => evaluate(Path::new(path)),
        _ => Err("--test-scenario FILEは他の起動引数と併用できません".into()),
    };
    let (report, exit) = match result {
        Ok((ok, steps)) => (
            json!({"schema_version": 1, "version": APP_VERSION,
            "backend": "isolated", "ok": ok, "steps": steps}),
            i32::from(!ok),
        ),
        Err(error) => (
            json!({"schema_version": 1, "version": APP_VERSION,
            "backend": "isolated", "ok": false, "error": error, "steps": []}),
            2,
        ),
    };
    let mut stdout = std::io::stdout().lock();
    if writeln!(stdout, "{report}")
        .and_then(|()| stdout.flush())
        .is_err()
    {
        return 2;
    }
    exit
}
