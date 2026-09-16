# 词典数据准备

pardon 的离线查词依赖两个本地 SQLite 词典，放在数据目录
`$PARDON_HOME/dict/`（缺省 `~/.local/share/pardon/dict/`）下：

| 文件 | 来源 | 方向 |
| --- | --- | --- |
| `ecdict.sqlite` | [ECDICT](https://github.com/skywind3000/ECDICT/releases) | 英→中 |
| `cedict.sqlite` | [CC-CEDICT](https://www.mdbg.net/chinese/dictionary?page=cedict) | 中→英 |

词典缺失不报错：`pardon` 会降级为空库并在 stderr 提示导入命令。
两个词典都装上，中英查词才能全覆盖。

先编译一次导入器（`pardon-import` 在 `pardon-core` 包里）：

```bash
cargo build --release -p pardon-core --bin pardon-import
```

## ECDICT（英→中，340 万词）

**release 不提供 CSV 打包**（官方资产为 sqlite/stardict/mdx/eudic/mobi），
推荐直接用 SQLite 版：

1. 下载 release: <https://github.com/skywind3000/ECDICT/releases>
   选 `ecdict-sqlite-28.zip`（206MB，或更新版本的 sqlite 包）。
2. 解压出 `ecdict.db`（官方库，表名 `stardict`，含 id/sw 附加列——
   导入器会跳过它们并重建我们自己的索引与词形反向表）。
3. 导入（先确保目标目录存在，导入器不会自动建目录）：

```bash
mkdir -p ~/.local/share/pardon/dict
cargo run -p pardon-core --release --bin pardon-import -- --ecdict-sqlite \
  ecdict.db ~/.local/share/pardon/dict/ecdict.sqlite
```

成功时输出 `imported <行数> entries into …`（全量约 340 万行，随版本略有
出入）。

**轻量备选（基础版 CSV，约 76 万词）**：官方仓库内的 `ecdict.csv` 是基础版，
直接走 CSV 模式：

```bash
curl -L -o ecdict.csv https://raw.githubusercontent.com/skywind3000/ECDICT/master/ecdict.csv
cargo run -p pardon-core --release --bin pardon-import -- \
  ecdict.csv ~/.local/share/pardon/dict/ecdict.sqlite
```

## CC-CEDICT（中→英，带拼音）

1. 下载: <https://www.mdbg.net/chinese/dictionary?page=cedict>
   （`cedict_1_0_ts_utf-8_mdbg.zip`）。
2. 解压出 `cedict_ts.u8`。
3. 导入（注意 `--cedict` 要放在输入文件之前）：

```bash
mkdir -p ~/.local/share/pardon/dict
cargo run -p pardon-core --release --bin pardon-import -- --cedict \
  cedict_ts.u8 ~/.local/share/pardon/dict/cedict.sqlite
```

## 导入器用法速查

```
pardon-import --ecdict-sqlite <官方ecdict.db> <out.sqlite>  # ECDICT release sqlite 版（推荐）
pardon-import <ecdict.csv> <out.sqlite>                     # ECDICT CSV（仓库基础版 76 万词）
pardon-import --cedict <cedict.u8> <out.sqlite>             # CC-CEDICT 模式
```

导入器会新建/覆盖目标 sqlite 文件并自建表与索引；父目录必须已存在。

## 用仓库 mini fixture 做冒烟验证

不想先下大文件？仓库自带两个极小样本
（`crates/core/tests/fixtures/`），几秒即可走通导入 → 查词全链路：

```bash
# 1. 导入两个 mini fixture（debug 构建即可；导入器不建目录，先 mkdir）
mkdir -p /tmp/pardon-smoke
cargo run -p pardon-core --bin pardon-import -- \
  crates/core/tests/fixtures/ecdict_mini.csv /tmp/pardon-smoke/ecdict.sqlite
# → imported 7 entries into /tmp/pardon-smoke/ecdict.sqlite

cargo run -p pardon-core --bin pardon-import -- \
  --cedict crates/core/tests/fixtures/cedict_mini.u8 /tmp/pardon-smoke/cedict.sqlite
# → imported 5 entries into /tmp/pardon-smoke/cedict.sqlite

# 2. 放到临时 PARDON_HOME 下，按真实目录结构摆放
mkdir -p /tmp/pardon-smoke-home/dict
cp /tmp/pardon-smoke/*.sqlite /tmp/pardon-smoke-home/dict/

# 3. 查词验证：英文走 ECDICT，中文走 CEDICT
PARDON_HOME=/tmp/pardon-smoke-home cargo run -p pardon-cli -- lookup run --json
# → {"found":true,"word":"run","phonetic":{"uk":"rʌn"},…,"source":"ecdict"}

PARDON_HOME=/tmp/pardon-smoke-home cargo run -p pardon-cli -- lookup 你好 --json
# → {"found":true,"word":"你好","phonetic":{"uk":"nǐ hǎo"},…,"source":"cedict"}
```

## 备注

- **数据许可**：ECDICT 与 CC-CEDICT 的数据**不在 pardon 仓库内分发**，
  许可归上游项目所有（CC-CEDICT 为 CC BY-SA 4.0；ECDICT 见其仓库声明）。
  本仓库只提供导入器，导入后的 sqlite 文件仅限本地使用，请勿再分发。
- **导入耗时**：全量 ECDICT（约 340 万行）目前是逐行 INSERT（尚未做事务
  批量优化），release 构建下也需要几分钟，属正常现象；CEDICT（约 12 万
  行）只要几秒。导入全程无网络请求，可放心离线进行。
- **拼音格式**：CEDICT 导入器同时兼容官方的方括号拼音
  (`繁體 简体 [pin1 yin1] /gloss/`) 和去括号的简写格式
  (`繁體 简体 pin1 yin1 gloss`)；查词输出会把数字声调（`ni3`）转为
  声调符号（`nǐ`）。
- `PARDON_HOME` 环境变量可整体替换数据根目录（词典在 `<home>/dict`，
  TTS 缓存在 `<home>/tts`），适合多套词典并行或把大词典放到别的盘。
