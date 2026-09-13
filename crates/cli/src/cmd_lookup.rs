//! `pardon lookup <word> [--json]`：离线词典查词（ECDICT/CEDICT）。

/// 查词并打印 WordCard JSON：`--json` 紧凑单行，否则 pretty 多行。
/// 返回退出码：0 命中 / 1 未命中（stdout 仍为合法 `{"found":false,…}`）。
pub fn run(word: &str, json: bool) -> anyhow::Result<i32> {
    let cfg = pardon_core::config::load()?;
    let pipeline = pardon_core::pipeline::Pipeline::from_config(&cfg)?;
    let card = pipeline.lookup(word);
    let out = if json {
        serde_json::to_string(&card)?
    } else {
        serde_json::to_string_pretty(&card)?
    };
    println!("{out}");
    Ok(i32::from(!card.found))
}
