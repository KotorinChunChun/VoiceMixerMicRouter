//! コマンドの世代管理と20ms制御ループ。RVCの遅い準備処理はセッションへ委ねる。

use super::super::{Command, CommandSender, Status};
use super::*;
use std::{
    path::PathBuf,
    sync::mpsc::{self, Receiver},
};

pub fn spawn(
    mut settings: Settings,
    directory: PathBuf,
) -> (
    CommandSender,
    Receiver<Status>,
    telemetry::Shared,
    thread::JoinHandle<()>,
) {
    let (tx, rx) = mpsc::channel();
    let route_request = Arc::new(AtomicU8::new(0));
    let sender = CommandSender {
        tx,
        route_request: route_request.clone(),
    };
    let (events, statuses) = mpsc::channel();
    let meters = telemetry::Shared::default();
    let latest = meters.clone();
    let worker = thread::spawn(move || {
        let mut session: Option<ResidentSession> = None;
        let mut output_holds = [false; 3];
        let mut desired: Option<(Mode, u64)> = None;
        let mut retry = Instant::now();
        let mut report = Instant::now();
        let mut failed = false;
        let mut deferred_configuration: Option<(Settings, Settings)> = None;
        'run: loop {
            let command = match rx.recv_timeout(Duration::from_millis(20)) {
                Ok(Command::Exit) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Ok(c) => Some(c),
                Err(_) => None,
            };
            for command in command.into_iter().chain(rx.try_iter()) {
                *latest.lock().unwrap() = None;
                if let Some(active) = &mut session {
                    active.meters.reset();
                }
                let mut applied = None;
                let result = match command {
                    Command::Sound(sound, device) => {
                        desired.get_or_insert((Mode::Direct, 0));
                        if let Some(s) = session.as_mut() {
                            s.request_sound(sound, device)
                        } else {
                            let _ = crate::sounds::play(sound, device);
                            Ok(())
                        }
                    }
                    Command::Hold(holds) => {
                        output_holds = holds;
                        desired.get_or_insert((Mode::Direct, 0));
                        failed = false;
                        retry = Instant::now();
                        session
                            .as_mut()
                            .map_or(Ok(()), |s| s.set_output_holds(holds))
                    }

                    Command::Set(mode, generation) => {
                        let mode = settings.available_mode(mode);
                        desired = Some((mode, generation));
                        failed = false;
                        retry = Instant::now();
                        if session.is_none() {
                            let _ = events.send(Status::new(
                                mode,
                                generation,
                                "starting",
                                format!("{}へ切替中", mode.label()),
                            ));
                        }
                        session.as_mut().map_or(Ok(()), |s| s.request_mode(mode))
                    }
                    Command::Configure(next, generation) => {
                        let mode = next.available_mode(desired.map_or(Mode::Direct, |d| d.0));
                        desired = Some((mode, generation));
                        let mut deferred = false;
                        let result = if let Some(s) = session.as_mut() {
                            if s.drain_target.is_some() {
                                Err("残音の送信完了後に設定を適用してください".into())
                            } else if next.rvc_enabled
                                && s.preparation.pending()
                                && next.rvc_parameters != s.settings.rvc_parameters
                            {
                                let mut immediate = (*next).clone();
                                immediate.rvc_parameters = s.settings.rvc_parameters.clone();
                                let result = s.configure(&immediate);
                                if result.is_ok() {
                                    deferred_configuration =
                                        Some(((*next).clone(), settings.clone()));
                                    deferred = true;
                                }
                                result
                            } else {
                                s.configure_request(&next, &settings).map(|completed| {
                                    deferred = !completed;
                                })
                            }
                        } else {
                            ResidentSession::new_with_request(
                                &next,
                                &directory,
                                mode,
                                route_request.clone(),
                            )
                            .map(|s| {
                                session = Some(s);
                            })
                        };
                        if !deferred {
                            applied = Some(result.is_ok());
                        }
                        if result.is_ok() && !deferred {
                            settings = *next;
                        }
                        result
                    }
                    Command::ReleaseRvc(generation) => {
                        desired = Some((Mode::Direct, generation));
                        session
                            .as_mut()
                            .map_or(Ok(()), ResidentSession::release_rvc)
                    }
                    Command::Restart(generation) => {
                        let mode = desired.map_or(Mode::Direct, |d| d.0);
                        desired = Some((mode, generation));
                        session.as_mut().map_or(Ok(()), |s| {
                            s.set_mode(Mode::Muted)?;
                            s.unload_rvc()?;
                            s.set_mode(mode)
                        })
                    }
                    Command::Exit => break 'run,
                };
                let (mode, generation) = desired.unwrap();
                let mut update = Status::new(
                    mode,
                    generation,
                    session.as_ref().map_or("starting", ResidentSession::phase),
                    session.as_ref().map_or_else(
                        || format!("{} → {}", mode.label(), settings.cable_name),
                        ResidentSession::status_message,
                    ),
                );
                update.applied = applied;
                if let Err(error) = result {
                    update.phase = "error".into();
                    update.message = error;
                    failed = applied != Some(false);
                    retry = Instant::now() + Duration::from_secs(10);
                }
                if let Some(s) = &session {
                    update.diagnostics = Some(s.snapshot());
                }
                if session.is_some() || applied.is_some() || update.phase == "error" {
                    let _ = events.send(update);
                }
                report = Instant::now();
            }
            let Some((mode, generation)) = desired else {
                continue;
            };
            if session.is_none() && Instant::now() >= retry {
                let _ = events.send(Status::new(
                    mode,
                    generation,
                    "starting",
                    format!("{} 準備中", mode.label()),
                ));
                match ResidentSession::new_with_request(
                    &settings,
                    &directory,
                    mode,
                    route_request.clone(),
                ) {
                    Ok(s) => {
                        session = Some(s);
                        failed = false;
                        report = Instant::now() - Duration::from_secs(2);
                    }
                    Err(e) => {
                        let _ = events.send(Status::new(mode, generation, "error", e));
                        retry = Instant::now() + Duration::from_secs(10);
                    }
                }
            }
            if let Some(active) = session.as_mut() {
                let previous_phase = active.phase();
                // 再試行待ちにも全イベントを消費し、旧instanceの通知を有効化前に捨てる。
                let mut result = if active.output_holds != output_holds {
                    active.set_output_holds(output_holds)
                } else {
                    Ok(())
                }
                .and_then(|_| active.tick_audio())
                .and_then(|_| active.advance_sound())
                .and_then(|_| active.advance_drain());
                let prepared = active.poll_rvc_prepare().and_then(|completed| {
                    active.retry_rvc()?;
                    Ok(completed)
                });
                let completed = matches!(prepared, Ok(true));
                if completed {
                    report = Instant::now() - Duration::from_secs(2);
                }
                if let Some(response) = active.configuration_response.take() {
                    settings = response.settings;
                    let mut update =
                        Status::new(mode, generation, active.phase(), response.message);
                    update.applied = Some(response.applied);
                    update.diagnostics = Some(active.snapshot());
                    let _ = events.send(update);
                }
                if !active.preparation.pending() {
                    if let Some((next, previous)) = deferred_configuration.take() {
                        let result = active.configure_request(&next, &previous);
                        if matches!(result, Ok(false)) {
                            // 設定トランザクションの新モデル／旧モデル復元を待つ。
                            report = Instant::now() - Duration::from_secs(2);
                        } else {
                            let mut update = Status::new(
                                mode,
                                generation,
                                active.phase(),
                                format!("{} 設定を適用しました", mode.label()),
                            );
                            update.applied = Some(result.is_ok());
                            match result {
                                Ok(_) => settings = next,
                                Err(error) => {
                                    let rollback = active.configure(&previous);
                                    update.phase = "error".into();
                                    update.message =
                                        rollback.err().map_or(error.clone(), |rollback| {
                                            format!("{error} / 旧設定への復旧にも失敗: {rollback}")
                                        });
                                }
                            }
                            update.diagnostics = Some(active.snapshot());
                            let _ = events.send(update);
                        }
                    }
                }
                if let Err(error) = prepared {
                    result = Err(error);
                }
                if failed && Instant::now() >= retry {
                    result = active
                        .repair()
                        .and_then(|_| active.refresh_monitor_target())
                        .and_then(|_| active.tick_audio());
                } else if !failed {
                    result = result.and_then(|_| active.refresh_monitor_target());
                }
                match result {
                    // 一般デバイス・循環検査の異常は、次の修復を通るまで解除しない。
                    // イベントは上で消化済みなので、過去の通知は溜まらない。
                    Ok(()) if failed && Instant::now() < retry => continue,
                    Ok(()) => {
                        failed = false;
                        let latency = active.measure_delay();
                        if let Some(mut snapshot) =
                            active
                                .meters
                                .publish(Instant::now(), generation, mode, latency)
                        {
                            if ![active.output, active.voice_input, active.monitor]
                                .into_iter()
                                .any(|id| active.session.backend().nodes.is_active(id))
                            {
                                if let Some(sample) = snapshot.samples.last_mut() {
                                    sample.output = Some(0.0);
                                }
                            }
                            *latest.lock().unwrap() = Some(snapshot);
                        }
                        if report.elapsed() >= Duration::from_secs(1)
                            || previous_phase != active.phase()
                        {
                            let mut update = Status::new(
                                mode,
                                generation,
                                active.phase(),
                                active.status_message(),
                            );
                            update.diagnostics = Some(active.snapshot());
                            if let Some(snapshot) = &*latest.lock().unwrap() {
                                update.diagnostics.as_mut().unwrap()["overlay_telemetry"] = serde_json::json!({
                                    "latency_estimate_ms":snapshot.latency_ms,
                                    "outputs_enabled":snapshot.outputs_enabled,
                                    "samples":snapshot.samples.iter().map(|s| serde_json::json!({"input_peak":s.input,"output_peak":s.output,"output_peaks":s.outputs})).collect::<Vec<_>>()
                                });
                            }
                            let _ = events.send(update);
                            report = Instant::now();
                        }
                    }
                    Err(error) => {
                        *latest.lock().unwrap() = None;
                        active.meters.reset();
                        active.delay.reset();
                        if !failed || Instant::now() >= retry {
                            let _ = events.send(Status::new(mode, generation, "error", error));
                            failed = true;
                            retry = Instant::now() + Duration::from_secs(10);
                        }
                    }
                }
            }
        }
        if let Some(mut active) = session {
            // 生成中のinstanceを残してBackendを破棄しない。準備と停止を直列化する。
            let _ = active.preparation.finish();
            let _ = active.disable_rvc_route();
            let _ = active.session.backend().nodes.stop_node(active.rvc);
            let _ = active.session.stop();
        }
    });
    (sender, statuses, meters, worker)
}
