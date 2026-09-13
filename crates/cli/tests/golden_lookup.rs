//! Golden 契约测试：`pardon lookup` 的 stdout JSON 与退出码。
//!
//! PARDON_HOME 指向 core 测试仓库内的 pip_home（mini fixture 导入产物，
//! 进 git）；PARDON_CONFIG 指向不存在路径以隔离用户级 config.toml
//! （缺失 → 内置默认配置）。

use assert_cmd::Command;

fn pardon() -> Command {
    let mut c = Command::cargo_bin("pardon").unwrap();
    let core_tests = env!("CARGO_MANIFEST_DIR").to_string() + "/../core/tests";
    c.env("PARDON_HOME", core_tests.clone() + "/pip_home");
    c.env("PARDON_CONFIG", core_tests + "/pip_home/no-such-config.toml");
    c
}

#[test]
fn lookup_hit_exit0_matches_golden() {
    let out = pardon().args(["lookup", "run", "--json"]).unwrap();
    assert!(out.status.success(), "expected exit 0, got status: {:?}", out.status);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout.trim_end(),
        include_str!("golden/lookup_run.json").trim_end()
    );
}

#[test]
fn lookup_miss_exit1_with_suggestions() {
    // mini 词典无 "helo" 词条；建议词按编辑距离给出 hello
    pardon()
        .args(["lookup", "helo", "--json"])
        .assert()
        .code(1)
        .stdout(predicates::str::contains("\"found\":false"))
        .stdout(predicates::str::contains("\"suggestions\":[\"hello\"]"));
}
