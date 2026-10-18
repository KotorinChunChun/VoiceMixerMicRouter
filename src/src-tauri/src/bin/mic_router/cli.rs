//! 起動引数、単一起動、初期化と診断。
#[cfg(windows)]
use super::ui;
use super::{
    actions,
    config::{self, Mode, Settings},
    engine,
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[cfg(test)]
#[path = "cli/tests.rs"]
mod tests;

#[derive(Debug, PartialEq, Eq)]
enum Invocation {
    Help,
    Version,
    Run(Options),
}

#[derive(Debug, PartialEq, Eq)]
struct Options {
    path: PathBuf,
    init: Option<PathBuf>,
    check: Option<Mode>,
    mode: Option<Mode>,
    action: Option<usize>,
    status: bool,
}
impl Options {
    fn command(&self) -> usize {
        self.action.unwrap_or(match self.mode {
            Some(Mode::Direct) => actions::DIRECT,
            Some(Mode::Rvc) => actions::RVC,
            Some(Mode::SpeakerRvc) => actions::SPK_RVC,
            Some(Mode::Muted) => actions::MUTED,
            None => actions::SHOW,
        })
    }
}

/// OS状態を参照せずに解析する。同種の指定は後勝ち、ヘルプと版表示は到達時に終了する。
fn parse(
    args: impl IntoIterator<Item = String>,
    default_path: PathBuf,
) -> Result<Invocation, String> {
    let mut options = Options {
        path: default_path,
        init: None,
        check: None,
        mode: None,
        action: None,
        status: false,
    };
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => {
                options.path = PathBuf::from(args.next().ok_or("--configの値が必要です")?)
            }
            "--init" => {
                options.init = Some(PathBuf::from(
                    args.next().ok_or("--initにはpluginsフォルダーが必要です")?,
                ))
            }
            "--check" => {
                options.check = Some(Mode::parse(&args.next().ok_or("--checkの値が必要です")?)?)
            }
            "--mode" => {
                options.mode = Some(Mode::parse(&args.next().ok_or("--modeの値が必要です")?)?)
            }
            "--toggle" => options.action = Some(actions::TOGGLE),
            "--stop" => options.action = Some(actions::EXIT),
            "--settings" => options.action = Some(actions::SETTINGS),
            "--release-rvc" => options.action = Some(actions::RELEASE_RVC),
            "--status" => options.status = true,
            "--version" => return Ok(Invocation::Version),
            "--help" => return Ok(Invocation::Help),
            _ => return Err(format!("不明な引数: {arg}")),
        }
    }
    if (options.init.is_some() as u8
        + options.check.is_some() as u8
        + options.status as u8
        + (options.mode.is_some() || options.action.is_some()) as u8)
        > 1
        || (options.mode.is_some() && options.action.is_some())
    {
        return Err("初期化・診断・状態確認・起動操作は一度に1つ指定してください".into());
    }
    Ok(Invocation::Run(options))
}

pub fn run() -> Result<(), String> {
    let default_path = std::env::current_exe()
        .map_err(|e| e.to_string())?
        .with_file_name("mic-settings.json");
    let options = match parse(std::env::args().skip(1), default_path)? {
        Invocation::Version => {
            println!("{}", config::APP_VERSION);
            return Ok(());
        }
        Invocation::Help => {
            println!("VoiceMixerMicRouter\n--config FILE\n--init PLUGINS_DIR: 設定を新規生成\n--mode direct|rvc|speaker_rvc|muted: 指定モードで起動／常駐中なら切替\n--toggle: 常駐中のモードを切替\n--stop: 常駐を終了\n--settings: 設定画面を開く（未起動なら起動）\n--release-rvc: RVCを終了してMICへ切替\n--status: 最新の状態JSON\n--check direct|rvc|speaker_rvc|muted: 10秒間の実デバイス診断（常駐停止時のみ）\n引数なし: 前回モードで常駐");
            return Ok(());
        }
        Invocation::Run(options) => options,
    };
    #[cfg(windows)]
    let command = options.command();
    let Options {
        path,
        init,
        check,
        mode,
        action,
        status,
    } = options;
    if let Some(plugins) = init {
        if path.exists() {
            return Err("設定が既にあります。既存設定は上書きしません".into());
        }
        Settings::detect(&plugins)?.save(&path)?;
        return Ok(());
    }
    let path = std::fs::canonicalize(path).map_err(|e| e.to_string())?;
    let directory = path.parent().ok_or("設定の保存先がありません")?;
    if status {
        println!(
            "{}",
            std::fs::read_to_string(directory.join("status.json")).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    #[cfg(windows)]
    if check.is_none() {
        if ui::send_existing(command)? {
            return Ok(());
        }
        if action.is_some() && action != Some(actions::SETTINGS) {
            return Err("マイク常駐は起動していません".into());
        }
    }
    // 起動スクリプトを連打しても同じ経路を二重起動しない。
    let mut lock_options = std::fs::OpenOptions::new();
    lock_options
        .create(true)
        .truncate(false)
        .read(true)
        .write(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        lock_options.share_mode(0);
    }
    let _instance = match lock_options.open(directory.join("instance.lock")) {
        Ok(file) => file,
        Err(e) => {
            #[cfg(windows)]
            if check.is_none() {
                for _ in 0..100 {
                    std::thread::sleep(Duration::from_millis(50));
                    if ui::send_existing(command)? {
                        return Ok(());
                    }
                }
            }
            return Err(format!(
                "既に起動中、または設定フォルダーに書き込めません: {e}"
            ));
        }
    };
    let settings = Settings::load(&path)?;
    let Some(mode) = check else {
        return super::resident::run(settings, path, mode, action == Some(actions::SETTINGS));
    };
    let mut session = engine::start(&settings, directory, mode)?;
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(10) {
        session.tick().map_err(|e| e.to_string())?;
        std::thread::sleep(Duration::from_millis(25));
    }
    let report = engine::diagnostics(&session);
    session.stop().map_err(|e| e.to_string())?;
    config::write_json(
        &directory.join(format!("check-{}.json", mode.label())),
        &report,
    )?;
    Ok(())
}
