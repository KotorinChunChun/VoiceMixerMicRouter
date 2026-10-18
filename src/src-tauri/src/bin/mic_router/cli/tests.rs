//! 起動引数の互換性を、実デバイスや常駐プロセスに触れずに確認する。

use super::*;

fn parse_args(args: &[&str]) -> Result<Invocation, String> {
    parse(
        args.iter().map(|arg| (*arg).to_owned()),
        PathBuf::from("既定/mic-settings.json"),
    )
}

fn options(args: &[&str]) -> Options {
    match parse_args(args).unwrap() {
        Invocation::Run(options) => options,
        invocation => panic!("通常起動が必要です: {invocation:?}"),
    }
}

#[test]
fn modes_and_actions_preserve_existing_process_commands() {
    let initial = options(&[]);
    assert_eq!(initial.path, PathBuf::from("既定/mic-settings.json"));
    assert_eq!(initial.mode, None);
    assert_eq!(initial.command(), actions::SHOW);
    for (mode, expected, command) in [
        ("direct", Mode::Direct, actions::DIRECT),
        ("rvc", Mode::Rvc, actions::RVC),
        ("speaker_rvc", Mode::SpeakerRvc, actions::SPK_RVC),
        ("spk-rvc", Mode::SpeakerRvc, actions::SPK_RVC),
        ("muted", Mode::Muted, actions::MUTED),
    ] {
        let parsed = options(&["--mode", mode]);
        assert_eq!(parsed.mode, Some(expected));
        assert_eq!(parsed.command(), command);
        assert_eq!(options(&["--check", mode]).check, Some(expected));
    }
    for (flag, command) in [
        ("--toggle", actions::TOGGLE),
        ("--stop", actions::EXIT),
        ("--settings", actions::SETTINGS),
        ("--release-rvc", actions::RELEASE_RVC),
    ] {
        let parsed = options(&[flag]);
        assert_eq!(parsed.action, Some(command));
        assert_eq!(parsed.command(), command);
    }
}

#[test]
fn conflicting_operation_categories_keep_the_existing_error() {
    let operations: [&[&str]; 5] = [
        &["--init", "plugins"],
        &["--check", "direct"],
        &["--status"],
        &["--mode", "rvc"],
        &["--settings"],
    ];
    for (index, first) in operations.iter().enumerate() {
        for second in operations.iter().skip(index + 1) {
            for (first, second) in [(first, second), (second, first)] {
                let args: Vec<_> = first.iter().chain(second.iter()).copied().collect();
                assert_eq!(
                    parse_args(&args).unwrap_err(),
                    "初期化・診断・状態確認・起動操作は一度に1つ指定してください",
                    "{args:?}",
                );
            }
        }
    }
}

#[test]
fn missing_values_unknown_arguments_and_invalid_modes_keep_the_existing_errors() {
    for (flag, error) in [
        ("--config", "--configの値が必要です"),
        ("--init", "--initにはpluginsフォルダーが必要です"),
        ("--check", "--checkの値が必要です"),
        ("--mode", "--modeの値が必要です"),
    ] {
        assert_eq!(parse_args(&[flag]).unwrap_err(), error);
    }
    assert_eq!(
        parse_args(&["--unknown"]).unwrap_err(),
        "不明な引数: --unknown",
    );
    for flag in ["--mode", "--check"] {
        for value in ["invalid", "--stop"] {
            assert_eq!(
                parse_args(&[flag, value]).unwrap_err(),
                "モードは direct、rvc、speaker_rvc、muted です",
            );
        }
    }
    // 値としての先頭ハイフンは従来どおり許可し、別オプションと解釈しない。
    assert_eq!(
        options(&["--config", "--help"]).path,
        PathBuf::from("--help")
    );
    assert_eq!(
        options(&["--init", "--version"]).init,
        Some(PathBuf::from("--version")),
    );
}

#[test]
fn help_and_version_return_at_their_position_before_later_validation() {
    for (flag, expected) in [
        ("--help", Invocation::Help),
        ("--version", Invocation::Version),
    ] {
        assert_eq!(parse_args(&[flag, "--unknown"]).unwrap(), expected);
        assert_eq!(
            parse_args(&["--status", "--mode", "rvc", flag, "--config"]).unwrap(),
            expected,
        );
        assert_eq!(
            parse_args(&["--unknown", flag]).unwrap_err(),
            "不明な引数: --unknown",
        );
        assert_eq!(
            parse_args(&["--mode", "invalid", flag]).unwrap_err(),
            "モードは direct、rvc、speaker_rvc、muted です",
        );
    }
    assert_eq!(
        parse_args(&["--help", "--version"]).unwrap(),
        Invocation::Help
    );
    assert_eq!(
        parse_args(&["--version", "--help"]).unwrap(),
        Invocation::Version
    );
}

#[test]
fn repeated_options_keep_the_last_value_without_reordering_config() {
    assert_eq!(
        options(&["--mode", "rvc", "--mode", "direct"]).mode,
        Some(Mode::Direct),
    );
    assert_eq!(
        options(&["--check", "muted", "--check", "speaker_rvc"]).check,
        Some(Mode::SpeakerRvc),
    );
    assert_eq!(
        options(&["--init", "first", "--init", "last"]).init,
        Some(PathBuf::from("last")),
    );
    assert_eq!(
        options(&["--toggle", "--stop", "--release-rvc", "--settings"]).action,
        Some(actions::SETTINGS),
    );
    assert!(options(&["--status", "--status"]).status);
    for args in [
        [
            "--config",
            "先/mic.json",
            "--mode",
            "rvc",
            "--config",
            "後/mic.json",
        ],
        [
            "--mode",
            "rvc",
            "--config",
            "先/mic.json",
            "--config",
            "後/mic.json",
        ],
    ] {
        let parsed = options(&args);
        assert_eq!(parsed.path, PathBuf::from("後/mic.json"));
        assert_eq!(parsed.mode, Some(Mode::Rvc));
    }
}

#[test]
fn management_start_arguments_preserve_origin_and_suppress_existing_window_show() {
    for origin in [
        "standalone",
        "host_manual",
        "direct_task",
        "via_kotorintoys",
    ] {
        let value = options(&["--startup", "--launch-origin", origin]);
        assert!(value.startup);
        assert_eq!(value.launch_origin, origin);
        assert_eq!(value.command(), 0);
    }
    assert!(parse_args(&["--launch-origin"]).is_err());
    assert!(parse_args(&["--launch-origin", "unknown"]).is_err());
    assert_eq!(
        options(&["--startup", "--settings"]).command(),
        actions::SETTINGS
    );
}
