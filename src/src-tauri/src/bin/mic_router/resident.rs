//! UI操作、設定適用の応答と音声ワーカーを結ぶ常駐ループ。
use super::{
    config::{self, Mode, Settings},
    engine,
};
#[cfg(windows)]
use super::{settings_ui, ui};
use std::{collections::VecDeque, path::PathBuf, time::Duration};

#[cfg(windows)]
#[derive(Clone, Copy)]
enum ApplyOrigin {
    Editor,
    Shortcut(usize),
}
#[cfg(windows)]
struct PendingSettings {
    settings: Settings,
    origin: ApplyOrigin,
}

/// 適用中の操作は保留するが、音声をMICへ戻す・止める操作は待たせない。
fn wait_for_configuration(action: usize, pending: bool) -> bool {
    pending
        && !matches!(
            action,
            crate::actions::SETTINGS
                | crate::actions::EXIT
                | crate::actions::DIRECT
                | crate::actions::MUTED
        )
}

fn cancel_queued_modes_on_escape(action: usize, deferred: &mut VecDeque<usize>) {
    use crate::actions::*;
    if matches!(action, DIRECT | MUTED) {
        deferred.retain(|queued| !matches!(*queued, RVC | SPK_RVC | TOGGLE | RETRY | RELEASE_RVC));
    }
}

// 設定の応答は、後続のモード変更で世代が進んでも必ず消化する。
// モデル設定を保留した応答は、その時点の最新世代で返る。
fn take_applied_settings<T>(
    pending: &mut Option<(u64, T)>,
    status: &engine::Status,
) -> Option<(T, bool)> {
    let applied = status.applied?;
    if pending
        .as_ref()
        .is_some_and(|(generation, _)| status.generation >= *generation)
    {
        pending.take().map(|(_, value)| (value, applied))
    } else {
        None
    }
}

#[cfg(test)]
mod resident_tests {
    use super::*;

    #[test]
    fn latest_mic_or_stop_cancels_older_mode_choices_but_keeps_output_changes() {
        use crate::actions::*;
        for escape in [DIRECT, MUTED] {
            let mut queued = VecDeque::from([RVC, OUTPUT1, SPK_RVC, OVERLAY, TOGGLE, OUTPUT1]);
            cancel_queued_modes_on_escape(escape, &mut queued);
            assert_eq!(
                queued.into_iter().collect::<Vec<_>>(),
                [OUTPUT1, OVERLAY, OUTPUT1]
            );
        }
    }

    #[test]
    fn configuration_wait_preserves_mode_escape_and_idle_shortcuts() {
        use crate::actions::*;
        for action in [DIRECT, MUTED, SETTINGS, EXIT] {
            assert!(!wait_for_configuration(action, true));
        }
        for action in [
            OUTPUT1, OUTPUT2, MONITOR, OVERLAY, NOISE, RVC, SPK_RVC, TOGGLE,
        ] {
            assert!(wait_for_configuration(action, true));
            assert!(!wait_for_configuration(action, false));
        }
    }

    #[test]
    fn configuration_ack_survives_newer_mic_selection_without_consuming_other_events() {
        let mut pending = Some((2, "出力1をOFF"));
        let mut status = engine::Status::new(Mode::Direct, 3, "running", String::new());
        assert_eq!(take_applied_settings(&mut pending, &status), None);
        status.generation = 1;
        status.applied = Some(true);
        assert_eq!(take_applied_settings(&mut pending, &status), None);
        status.generation = 2;
        assert_eq!(
            take_applied_settings(&mut pending, &status),
            Some(("出力1をOFF", true))
        );
        assert!(pending.is_none());
        // RVC準備後の保留設定は、最新のモード世代で完了する。
        pending = Some((4, "RVCパラメーター"));
        status.generation = 5;
        status.applied = Some(false);
        assert_eq!(
            take_applied_settings(&mut pending, &status),
            Some(("RVCパラメーター", false))
        );
        assert!(pending.is_none());
    }
}

#[cfg(windows)]
pub fn run(
    mut settings: Settings,
    path: PathBuf,
    initial: Option<Mode>,
    open_settings: bool,
    silent: bool,
    launch_origin: String,
) -> Result<(), String> {
    let management = crate::management::Endpoint::start()?;
    let directory = path.parent().unwrap().to_path_buf();
    let mut desired = settings.available_mode(
        initial
            .or(settings.startup_mode)
            .unwrap_or(settings.last_mode),
    );
    let mut generation = 1;
    let display_mode = if silent {
        config::OverlayMode::Off
    } else {
        settings.overlay_mode
    };
    let mut ui = ui::Ui::new(
        desired,
        &settings.hotkeys,
        display_mode,
        settings.startup_mode,
        settings.monitoring,
        settings.noise_suppression(),
    )?;
    ui.output_labels = settings.output_labels.clone();
    ui.rvc_enabled = settings.rvc_enabled;
    if let Err(error) = ui.set_hotkeys(&settings.hotkeys) {
        ui.notify(&error);
    }
    ui.cable_enabled = settings.cable_enabled;
    ui.voice_input_enabled = settings.voice_input_enabled;
    ui.set_display(
        display_mode,
        settings.monitoring,
        settings.noise_suppression(),
    )?;
    let mut window = if open_settings {
        Some(settings_ui::SettingsWindow::new(&settings, desired)?)
    } else {
        None
    };
    let mut held = [false; 3];
    let mut pending: Option<(u64, PendingSettings)> = None;
    let mut deferred = VecDeque::new();
    let (commands, events, meters, worker) = engine::spawn(settings.clone(), directory.clone());
    let mut current = engine::Status::new(
        desired,
        generation,
        "starting",
        format!("{} 準備中（無音）", desired.label()),
    );
    let record = |state: &engine::Status| {
        config::write_json(
            &directory.join("status.json"),
            &serde_json::json!({
                "pid":std::process::id(),"updated_utc":chrono::Utc::now(),"status":state
            }),
        )
    };
    record(&current)?;
    commands
        .send(engine::Command::Set(desired, generation))
        .map_err(|e| e.to_string())?;
    let result = (|| -> Result<(), String> {
        loop {
            for event in management.poll() {
                let blocked = window.is_some() || pending.is_some();
                match event {
                    crate::management::Event::Stop(deadline) => {
                        if std::time::Instant::now() < deadline && !blocked {
                            return Ok(());
                        }
                    }
                    crate::management::Event::Request(call) => {
                        if std::time::Instant::now() >= call.deadline {
                            continue;
                        }
                        let (mut result, mut detail) =
                            crate::management::validate(&call.request, blocked);
                        if result == "completed" && call.request.command == "show" {
                            if let Some(w) = &window {
                                w.show();
                            } else {
                                match settings_ui::SettingsWindow::new(&settings, desired) {
                                    Ok(w) => window = Some(w),
                                    Err(_) => {
                                        result = "failed";
                                        detail = "設定画面を表示できませんでした。";
                                    }
                                }
                            }
                        }
                        call.respond(result, detail, &launch_origin);
                    }
                }
            }
            ui.interaction_locked = window.is_some() || pending.is_some();
            ui.allow_mode_escape = window.is_none() && pending.is_some();
            let queued = if pending.is_none() {
                deferred.pop_front()
            } else {
                None
            };
            for action in queued.into_iter().chain(ui.poll()?) {
                if !settings.rvc_enabled
                    && matches!(action, ui::RVC | ui::SPK_RVC | ui::NOISE | ui::RELEASE_RVC)
                {
                    continue;
                }
                // 解放は設定反映待ちでも必ず処理し、送信が残るのを防ぐ。
                if let Some((index, pressed)) = crate::actions::ptt_event(action) {
                    if held[index] != pressed {
                        held[index] = pressed;
                        ui.output_holds = held;
                        commands
                            .send(engine::Command::Hold(held))
                            .map_err(|e| e.to_string())?;
                        commands
                            .send(engine::Command::Sound(
                                settings.switch_sounds[4 + index],
                                settings.sound_device_id.clone(),
                            ))
                            .map_err(|e| e.to_string())?;
                        let effective = settings.with_output_holds(held);
                        ui.cable_enabled = effective.cable_enabled;
                        ui.voice_input_enabled = effective.voice_input_enabled;
                        ui.set_display(
                            settings.overlay_mode,
                            effective.monitoring,
                            settings.noise_suppression(),
                        )?;
                    }
                    continue;
                }

                if wait_for_configuration(action, pending.is_some()) {
                    deferred.push_back(action);
                    continue;
                }
                cancel_queued_modes_on_escape(action, &mut deferred);
                match action {
                    ui::EXIT => return Ok(()),
                    ui::SETTINGS => {
                        if let Some(w) = &window {
                            w.show();
                        } else {
                            match settings_ui::SettingsWindow::new(
                                pending
                                    .as_ref()
                                    .map(|(_, p)| &p.settings)
                                    .unwrap_or(&settings),
                                desired,
                            ) {
                                Ok(mut w) => {
                                    if let Some((_, change)) = &pending {
                                        match change.origin {
                                            ApplyOrigin::Editor => w.busy(true),
                                            ApplyOrigin::Shortcut(_) => w.shortcut_pending(true),
                                        }
                                    }
                                    window = Some(w);
                                }
                                Err(e) => {
                                    ui.notify(&e);
                                    ui.show()?;
                                }
                            }
                        }
                    }
                    ui::DIRECT
                    | ui::RVC
                    | ui::SPK_RVC
                    | ui::MUTED
                    | ui::TOGGLE
                    | ui::RETRY
                    | ui::RELEASE_RVC => {
                        let next = settings.available_mode(match action {
                            ui::DIRECT | ui::RELEASE_RVC => Mode::Direct,
                            ui::RVC => Mode::Rvc,
                            ui::SPK_RVC => Mode::SpeakerRvc,
                            ui::MUTED => Mode::Muted,
                            ui::TOGGLE => desired.other(),
                            _ => desired,
                        });
                        if next == desired
                            && current.phase == "running"
                            && !matches!(action, ui::RETRY | ui::RELEASE_RVC)
                        {
                            ui.show()?;
                            continue;
                        }
                        desired = next;
                        generation += 1;
                        current = engine::Status::new(
                            desired,
                            generation,
                            "starting",
                            format!("{} 準備中（無音）", desired.label()),
                        );
                        ui.update(desired, "starting", &current.message)?;
                        record(&current)?;
                        commands
                            .send(match action {
                                ui::RETRY => engine::Command::Restart(generation),
                                ui::RELEASE_RVC => engine::Command::ReleaseRvc(generation),
                                _ => engine::Command::Set(desired, generation),
                            })
                            .map_err(|e| e.to_string())?;
                        if !matches!(action, ui::RETRY | ui::RELEASE_RVC) {
                            let index = match desired {
                                Mode::Direct => 0,
                                Mode::Rvc => 1,
                                Mode::SpeakerRvc => 2,
                                Mode::Muted => 3,
                            };
                            commands
                                .send(engine::Command::Sound(
                                    settings.switch_sounds[index],
                                    settings.sound_device_id.clone(),
                                ))
                                .map_err(|e| e.to_string())?;
                        }
                    }
                    ui::OVERLAY => {
                        let mut candidate = settings.clone();
                        candidate.overlay_mode = candidate.overlay_mode.next();
                        match candidate.save(&path) {
                            Ok(()) => {
                                settings = candidate;
                                if let Some(w) = &mut window {
                                    w.sync_shortcut(ui::OVERLAY, &settings);
                                }
                                ui.set_display(
                                    settings.overlay_mode,
                                    settings.with_output_holds(held).monitoring,
                                    settings.noise_suppression(),
                                )?;
                            }
                            Err(e) => {
                                ui.notify(&e);
                                ui.show()?;
                            }
                        }
                    }
                    ui::OUTPUT1 | ui::OUTPUT2 | ui::MONITOR | ui::NOISE => {
                        let mut candidate = settings.clone();
                        match action {
                            ui::OUTPUT1 => candidate.cable_enabled = !candidate.cable_enabled,
                            ui::OUTPUT2 => {
                                candidate.voice_input_enabled = !candidate.voice_input_enabled
                            }
                            ui::NOISE => {
                                candidate.set_noise_suppression(!candidate.noise_suppression())
                            }
                            _ => candidate.monitoring = !candidate.monitoring,
                        }
                        match candidate.save(&path) {
                            Ok(()) => {
                                generation += 1;
                                commands
                                    .send(engine::Command::Configure(
                                        Box::new(candidate.clone()),
                                        generation,
                                    ))
                                    .map_err(|e| e.to_string())?;
                                pending = Some((
                                    generation,
                                    PendingSettings {
                                        settings: candidate,
                                        origin: ApplyOrigin::Shortcut(action),
                                    },
                                ));
                                if let Some(w) = &window {
                                    w.shortcut_pending(true);
                                }
                            }
                            Err(e) => {
                                ui.notify(&e);
                                ui.show()?;
                            }
                        }
                    }
                    20..=24 => {
                        if window.is_some() {
                            continue;
                        }
                        settings.startup_mode = match action {
                            21 => Some(Mode::Direct),
                            22 => Some(Mode::Rvc),
                            23 => Some(Mode::Muted),
                            24 => Some(Mode::SpeakerRvc),
                            _ => None,
                        };
                        ui.startup = settings.startup_mode;
                        settings.save(&path)?;
                    }
                    _ => {}
                }
            }
            if let Some(action) = window.as_mut().and_then(|w| w.poll()) {
                match action {
                    settings_ui::Action::Close => window = None,
                    settings_ui::Action::Preview(choice, device) => {
                        commands
                            .send(engine::Command::Sound(choice, device))
                            .map_err(|e| e.to_string())?;
                    }
                    settings_ui::Action::Save(_) if pending.is_some() => {
                        if let Some(w) = &window {
                            w.message("切替の反映が終わってから適用してください");
                        }
                    }
                    settings_ui::Action::Save(mut candidate) => {
                        candidate.last_mode = settings.last_mode;
                        let prepared = ui
                            .set_hotkeys_for_rvc(&candidate.hotkeys, candidate.rvc_enabled)
                            .and_then(|_| candidate.save(&path));
                        match prepared {
                            Ok(()) => {
                                generation += 1;
                                commands
                                    .send(engine::Command::Configure(candidate.clone(), generation))
                                    .map_err(|e| e.to_string())?;
                                pending = Some((
                                    generation,
                                    PendingSettings {
                                        settings: *candidate,
                                        origin: ApplyOrigin::Editor,
                                    },
                                ));
                                if let Some(w) = &mut window {
                                    w.busy(true);
                                }
                            }
                            Err(e) => {
                                if let Err(rollback) =
                                    ui.set_hotkeys_for_rvc(&settings.hotkeys, settings.rvc_enabled)
                                {
                                    ui.notify(&rollback);
                                }
                                if let Some(w) = &window {
                                    w.message(&e);
                                }
                            }
                        }
                    }
                }
            }
            for next in events.try_iter() {
                // MICへ戻す操作は設定適用中も通すため、設定の応答とモード表示を別に扱う。
                if let Some((change, applied)) = take_applied_settings(&mut pending, &next) {
                    let origin = change.origin;
                    let candidate = change.settings;
                    if applied {
                        settings = candidate;
                        ui.rvc_enabled = settings.rvc_enabled;
                        if let ApplyOrigin::Shortcut(action) = origin {
                            let index = match action {
                                ui::OUTPUT1 => Some(4),
                                ui::OUTPUT2 => Some(5),
                                ui::MONITOR => Some(6),
                                _ => None,
                            };
                            if let Some(index) = index {
                                commands
                                    .send(engine::Command::Sound(
                                        settings.switch_sounds[index],
                                        settings.sound_device_id.clone(),
                                    ))
                                    .map_err(|e| e.to_string())?;
                            }
                        }

                        desired = settings.available_mode(desired);
                        ui.startup = settings.startup_mode;
                        ui.output_labels = settings.output_labels.clone();
                        let effective = settings.with_output_holds(held);
                        ui.cable_enabled = effective.cable_enabled;
                        ui.voice_input_enabled = effective.voice_input_enabled;
                        ui.set_display(
                            settings.overlay_mode,
                            settings.with_output_holds(held).monitoring,
                            settings.noise_suppression(),
                        )?;
                        if let Some(w) = &mut window {
                            match origin {
                                ApplyOrigin::Editor => w.applied(&settings),
                                ApplyOrigin::Shortcut(action) => w.sync_shortcut(action, &settings),
                            }
                        } else {
                            ui.show()?;
                        }
                    } else {
                        let mut message = next.message.clone();
                        if let Err(e) = settings.save(&path) {
                            message.push_str(&format!(" / 設定の復元失敗: {e}"));
                        }
                        if matches!(origin, ApplyOrigin::Editor) {
                            if let Err(e) =
                                ui.set_hotkeys_for_rvc(&settings.hotkeys, settings.rvc_enabled)
                            {
                                message.push_str(&format!(" / キーの復元失敗: {e}"));
                            }
                        }
                        if let Some(w) = &mut window {
                            match origin {
                                ApplyOrigin::Editor => w.busy(false),
                                ApplyOrigin::Shortcut(_) => w.shortcut_pending(false),
                            }
                            w.message(&message);
                        } else {
                            ui.notify(&message);
                            ui.show()?;
                        }
                    }
                }
                if next.generation != generation {
                    continue;
                }
                if next.phase == "running" && settings.last_mode != next.mode {
                    settings.last_mode = next.mode;
                    settings.save(&path)?;
                }
                ui.update(next.mode, &next.phase, &next.message)?;
                record(&next)?;
                current = next;
            }
            ui.update_telemetry(meters.lock().unwrap().clone(), generation);
            if worker.is_finished() {
                return Err("音声ワーカーが終了しました".into());
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    })();
    current.phase = "stopping".into();
    current.message = "音声送信を終了しています".into();
    let _ = record(&current);
    let _ = ui.update(desired, "stopping", &current.message);
    let _ = commands.send(engine::Command::Exit);
    while !worker.is_finished() {
        let _ = ui.poll();
        std::thread::sleep(Duration::from_millis(25));
    }
    let _ = worker.join();
    current.phase = "stopped".into();
    current.message = "音声送信を停止しました".into();
    current.diagnostics = None;
    let _ = record(&current);
    result
}
#[cfg(not(windows))]
pub fn run(
    _settings: Settings,
    _path: PathBuf,
    _initial: Option<Mode>,
    _open_settings: bool,
    _silent: bool,
    _launch_origin: String,
) -> Result<(), String> {
    Err("Windows専用です".into())
}
