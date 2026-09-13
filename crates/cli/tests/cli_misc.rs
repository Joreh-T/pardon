//! 杂项 CLI 契约：`--version`、`config --init`、`speak`（TTS 测试开关）。
//!
//! 每个用例独立进程 + 专属临时目录（PARDON_HOME / PARDON_CONFIG），互不
//! 共享环境；`PARDON_TTS_DISABLE=1` 使 `Tts::speak` 不下载不播放直接 Ok，
//! 故 speak 用例不触真实网络/音频设备。

use assert_cmd::Command;

fn pardon() -> Command {
    Command::cargo_bin("pardon").unwrap()
}

#[test]
fn version_exit0_prints_name_and_version() {
    let out = pardon().arg("--version").unwrap();
    assert!(out.status.success(), "expected exit 0");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.starts_with("pardon 0.1.0"),
        "--version 应输出 `pardon 0.1.0` 前缀，got {stdout:?}"
    );
}

#[test]
fn config_init_writes_default_then_refuses_existing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    // 第一次：写入默认配置，stdout 报告路径，exit 0
    let out = pardon()
        .env("PARDON_CONFIG", &path)
        .args(["config", "--init"])
        .unwrap();
    assert!(
        out.status.success(),
        "首次 --init 应 exit 0，stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(path.to_str().unwrap()),
        "stdout 应含写出的路径，got {stdout:?}"
    );
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.contains(r#"default_engine = "youdao""#),
        "默认配置应含 default_engine = \"youdao\"，got {text:?}"
    );

    // 第二次：文件已存在 → exit 2，错误消息包含路径
    pardon()
        .env("PARDON_CONFIG", &path)
        .args(["config", "--init"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains(path.to_str().unwrap()));
}

#[test]
fn speak_with_test_switch_exits0_and_silent_stdout() {
    let dir = tempfile::tempdir().unwrap();
    pardon()
        .env("PARDON_HOME", dir.path()) // cache 目录 = $PARDON_HOME/tts
        .env("PARDON_TTS_DISABLE", "1") // 测试开关：不下载不播放
        .args(["speak", "hello", "--lang", "en"])
        .assert()
        .code(0)
        .stdout(predicates::str::is_empty());
}

#[test]
fn speak_defaults_to_auto_detect() {
    let dir = tempfile::tempdir().unwrap();
    pardon()
        .env("PARDON_HOME", dir.path())
        .env("PARDON_TTS_DISABLE", "1")
        .args(["speak", "你好"]) // 无 --lang → 默认 auto 检测为 zh
        .assert()
        .code(0)
        .stdout(predicates::str::is_empty());
}

#[test]
fn speak_unknown_lang_exits_2() {
    let dir = tempfile::tempdir().unwrap();
    pardon()
        .env("PARDON_HOME", dir.path())
        .env("PARDON_TTS_DISABLE", "1")
        .args(["speak", "hello", "--lang", "fr"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("unknown language"));
}

#[test]
fn speak_empty_text_exits_2() {
    let dir = tempfile::tempdir().unwrap();
    pardon()
        .env("PARDON_HOME", dir.path())
        .env("PARDON_TTS_DISABLE", "1")
        .args(["speak", ""]) // 空串 → 参数错误 exit 2，不应走到 TTS
        .assert()
        .code(2)
        .stderr(predicates::str::contains("empty text"));
}
