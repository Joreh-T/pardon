# pardon

Pardon my French — offline dictionary + LLM translation CLI, Chinese↔English,
built for nvim and terminals.

> M1 状态：CLI（lookup / translate / speak / config）+ 本地词典 + 引擎链
> （LLM → 有道 → 必应，自动降级）已可用；nvim 插件在本仓库 `nvim/` 目录
> （见下文 [nvim 集成](#nvim-集成)）。文档与打包仍在完善中。

## 安装

```bash
cargo install --path crates/cli
```

## 快速上手

### 1. 准备词典（离线查词）

查词依赖本地 SQLite 词典（ECDICT 英→中 + CC-CEDICT 中→英），下载与导入
步骤见 **[dicts/README.md](dicts/README.md)**。想先跑通流程可用其中的
mini fixture 冒烟一节，几秒即可。词典缺失时 `pardon` 不会报错，只是查词
降级为空结果并提示导入命令。

### 2. 查词

```bash
$ pardon lookup run --json
{"found":true,"word":"run","phonetic":{"uk":"rʌn"},"pos":[{"pos":"n.","gloss":["跑步"]},{"pos":"v.","gloss":["跑","运转"]}],"definition":["move fast","operate"],"exchange":{"past":"ran","pp":"run","ing":"running","third":"runs","lemma":"run"},"collins":3,"oxford":true,"tags":["zk","gk","cet4"],"source":"ecdict"}

$ pardon lookup 你好 --json
{"found":true,"word":"你好","phonetic":{"uk":"nǐ hǎo"},"pos":[{"pos":"","gloss":["you (informal)","hello"]}],"oxford":false,"source":"cedict"}
```

中文词走 CEDICT（带拼音，数字声调自动转声调符号），英文词走 ECDICT
（音标、词性分组释义、词形变化、Collins/Oxford 标签）。

### 3. 翻译

单词自动走离线词典路由（不联网）；句子走引擎链。注意：web 引擎
（有道/Bing）的免费端点已于 2026-09 失效，句子翻译需配置 LLM
provider（示例见下方[配置](#配置)节）；查词（离线词典）与发音不受影响。

```bash
$ echo "The quick brown fox jumps over the lazy dog." | pardon translate --stdin --stream
{"type":"meta","source":"en","target":"zh","engine":"auto"}
{"type":"delta","text":"…"}            # 流式增量（LLM 引擎时逐段输出）
{"type":"result","source":"en","target":"zh","engine":"…","text":"…","translation":"…"}
```

`--stream` 逐行输出 JSONL 事件（`meta` → `delta`… → `result`）；不加
`--stream` 可用 `--json` 拿单行结果 JSON。常用参数：

- `--engine llm|youdao|bing|auto`：指定引擎（指定后不做降级链）；
  `auto` 会按语言自动选择并允许失败降级到下一个引擎。
- `--source` / `--target en|zh|auto`：仅在显式指定 `--engine` 时生效；
  `--engine auto` 模式下方向由文本自动检测。
- `--timeout N`：整次操作超时秒数（默认 30；超时输出
  `{"type":"error","code":"timeout",…}` 并以退出码 124 结束）。
- 翻译一个单词（如 `pardon translate run`）优先离线词典卡片。

### 4. 朗读

```bash
pardon speak "hello world"        # --lang auto|en|zh
```

在线发音（有道 dictvoice）缓存到本地（`$PARDON_HOME/tts`），失败时回退
espeak-ng 离线合成。

## 配置

写出带注释的默认配置（文件已存在则拒绝覆盖）：

```bash
pardon config --init    # → ~/.config/pardon/config.toml
pardon config           # 只打印当前生效的配置路径
```

下面是一个覆盖全部四种 provider 写法的完整示例（OpenAI 官方、任意
OpenAI 兼容端点（以 DeepSeek 为例）、Anthropic、本地 Ollama）。密钥一律
走环境变量（`api_key_env`），不要把明文 key 写进配置文件：

```toml
default_engine = "llm"           # llm | youdao | bing（youdao/bing 免费端点已失效，句子翻译建议 llm）

[llm]
default_provider = "deepseek"

[[llm.providers]]
id = "deepseek"
type = "openai"                  # 任意 OpenAI 兼容端点
base_url = "https://api.deepseek.com/v1"
api_key_env = "DEEPSEEK_API_KEY"
model = "deepseek-chat"

[[llm.providers]]
id = "openai"
type = "openai"
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"
model = "gpt-4o-mini"

[[llm.providers]]
id = "claude"
type = "anthropic"
base_url = "https://api.anthropic.com"
api_key_env = "ANTHROPIC_API_KEY"
model = "claude-sonnet-4-5"

[[llm.providers]]
id = "local"
type = "ollama"                  # 需本机 11434 端口运行 Ollama
base_url = "http://127.0.0.1:11434/v1"
model = "qwen2.5:7b"
```

要点：

- `default_engine = "llm"` 时，`default_provider` 必须指向某个
  `providers[].id`，否则配置校验失败。
- 每个 provider 的 `base_url`、`model` 必填；`id` 唯一。
- `type = "ollama"` 无需密钥；其余类型需要 `api_key_env`（或
  `api_key`，明文写进配置文件需自担风险，优先级高于环境变量）。
- `default_engine = "youdao" | "bing"` 时不需要任何配置，但其免费端点
  已于 2026-09 失效（见上文[翻译](#3-翻译)节），句子翻译建议配置 LLM。

### 自定义提示词

每个 provider 可用 `system_prompt` / `user_prompt_template` 覆盖内置提示
词；模板占位符 `{text}`（原文）、`{source}` / `{target}`（渲染为语言显
示名，如 `English` / `Chinese (Simplified)`）：

```toml
[[llm.providers]]
id = "deepseek"
type = "openai"
base_url = "https://api.deepseek.com/v1"
api_key_env = "DEEPSEEK_API_KEY"
model = "deepseek-chat"
system_prompt = "你是专业译者，只输出译文，不要解释。"
user_prompt_template = "把这句{source}翻成{target}：{text}"
```

### 环境变量

| 变量 | 作用 |
| --- | --- |
| `PARDON_HOME` | 数据根目录（词典 `<home>/dict`、TTS 缓存 `<home>/tts`）；缺省 `~/.local/share/pardon` |
| `PARDON_CONFIG` | 配置文件路径覆盖；缺省 `~/.config/pardon/config.toml` |

## nvim 集成

nvim 插件随本仓库发布（`nvim/` 目录）：`:Pardon` 查光标下的词、
`:PardonTranslate` 翻译选区或光标词，词卡与译文在浮动窗口展示（翻译支
持流式渲染），`<Plug>` 键位由用户自行映射。安装与配置说明见
[nvim/README.md](nvim/README.md)。

## M2 — 复制即翻译（pardond）

常驻 daemon 监听剪贴板，复制即翻译并弹桌面通知；niri/WM 快捷键经
`pardon trigger` 唤起。`pardon lookup/translate/speak` 不依赖 daemon（内嵌直查）。

### 运行（推荐：systemd user service）

    cargo install --path crates/daemon --bin pardond   # 或与 pardon 一起安装
    mkdir -p ~/.config/systemd/user
    cp dist/systemd/pardon.service ~/.config/systemd/user/
    systemctl --user daemon-reload && systemctl --user enable --now pardon

无 systemd 场景：`pardon daemon start`（日志 `~/.local/share/pardon/log/pardond.log`）/ `pardon daemon stop`。

依赖：`wl-clipboard`（wl-paste/wl-copy，剪贴板监听与读写）；通知服务（mako/dunst）。
缺失时 daemon 自动降级为纯 HTTP 触发口模式（`pardon status` 可见）。

### niri 快捷键

```kdl
binds {
    Mod+T { spawn "pardon" "trigger" "selection"; }
    Mod+Shift+T { spawn "pardon" "trigger" "clipboard"; }
}
```

（GNOME 等桌面：系统设置 → 自定义快捷键 → 命令 `pardon trigger selection`。）

### HTTP 触发口（localhost only）

    curl -s http://127.0.0.1:7377/status
    curl -s -X POST http://127.0.0.1:7377/translate -H 'content-type: application/json' -d '{"text":"hello world"}'
    curl -s http://127.0.0.1:7377/trigger/selection

JSON 说明：响应中的可选项在缺省时直接省略键（如 trigger 响应未产生译文时
没有 `translation` 键），而非输出显式 `null`——消费方应把缺失键当作 `null`。

安全说明：只绑定回环地址、无鉴权——本地任意进程都可触发/关停（用户级信任域，
与剪贴板本身同权限级）。改端口见下方配置。

### [daemon] 配置（~/.config/pardon/config.toml，均可省略）

| 键 | 默认 | 说明 |
|----|------|------|
| http_bind | "127.0.0.1:7377" | HTTP 触发口（仅回环地址；PARDON_HTTP_BIND 环境变量可覆盖） |
| auto_translate | true | 复制即翻译总开关 |
| max_text_bytes | 5120 | 自动翻译文本上限（字节），超过忽略 |
| dedup_window_ms | 10000 | 同内容去重时间窗（防回环） |
| copy_translation | false | 译文自动写回剪贴板（写回前登记防回环，不会 ping-pong）；翻译进行期间的其他复制会被译文覆盖（防回环只防循环，不防丢失） |
| notify_timeout_ms | 5000 | 通知显示时长 |
| show_word_badge | false | 词卡通知显示学习徽章行（柯林斯星级 · 牛津核心 · 考试标签） |
| popup | false | 翻译结果优先 GUI 弹窗展示（无 GUI 订阅时回退桌面通知，见 [M3](#m3--gui-pardon-gui)） |

### 手动测试矩阵（Niri + mako/dunst）

1. 启动后复制一个英文单词（如 `run`）→ 通知显示词卡（词性+释义）
2. 复制整句 → 通知显示 LLM 译文（需配置 LLM provider，见上文引擎章节）
3. 立即再复制同样内容 → 无第二条通知（去重）
4. 开 `copy_translation = true` 后复制整句 → 剪贴板内容变为译文，可直接粘贴；
   且不会触发反向翻译（防回环）
5. `Mod+T` 选中一段文字按快捷键 → 通知弹出
6. 复制一张图片 → 无事件（非文本过滤）；随后复制文本 → 正常
7. `systemctl --user restart niri`（合成器重启）→ daemon 自动恢复监听（断线重连）
8. `pardon status` / `pardon daemon stop`

## M3 — GUI（pardon-gui）

Tauri 2 GUI（`gui/` 目录，独立 cargo 项目）：快捷键弹窗、主窗口（查词/翻译/
历史）、系统托盘、设置界面。GUI 是纯 IPC 客户端——所有翻译/词典/历史数据
经下方 UDS 协议向 pardond 请求，自身不内嵌翻译逻辑（可替换客户端）。

### 构建与运行

前置系统依赖（Debian/Ubuntu；其余发行版装对应 webkit2gtk-4.1 等价包）：

```bash
sudo apt install libwebkit2gtk-4.1-dev build-essential libxdo-dev librsvg2-dev libssl-dev
```

构建安装：

```bash
cd gui && npm install && npm run build
cd src-tauri && cargo install --path . --bin pardon-gui   # 或 cargo run 直接调试
```

运行 `pardon-gui`（加 `--settings` 启动即开设置窗口）。需 pardond 在跑；
主窗口检测到未连接时会提示并给出「一键启动」按钮。

### 开机自启（niri）

```kdl
spawn-at-startup "pardon-gui"
```

不配自启也可用：托盘「打开主窗口/设置」在 GUI 未运行时会拉起
（`pardon-gui --main` / `--settings`）。

### 弹窗模式（popup）

`[daemon] popup = true`（默认 false，通知模式行为不变；也可在设置界面切换）。
开启后翻译结果改用 GUI 弹窗展示：词卡带音标/词形，内容高度自适应
（宽固定 420），Esc 或失焦隐藏；隐藏时的位置记忆在
`~/.local/share/pardon/gui-state.json`，下次在原位弹出。无 GUI 订阅时自动
回退桌面通知，GUI 退出/断开即恢复通知——两条通路永不低于 M2 的可用性。

### 翻译历史

```bash
pardon history                 # 最近 20 条（新→旧，单行人类可读）
pardon history --limit 5
pardon history --json          # compact JSON 数组
pardon history --clear
```

存储 `~/.local/share/pardon/history.sqlite`（`PARDON_HOME` 可覆盖），上限
1000 条、超出自动裁剪最旧。CLI translate、daemon 剪贴板/触发翻译、GUI 主窗
口翻译均记录；`origin` 字段区分来源：`auto`（剪贴板）/ `trigger`（快捷键）/
`cli` / `gui`。

### UDS IPC 协议（GUI 契约）

套接字路径：`$XDG_RUNTIME_DIR/pardon/ipc.sock`（`PARDON_IPC_SOCK` 可覆盖；
无 XDG 时兜底 `/tmp/pardon/ipc.sock`）。socket 文件权限 0600，daemon 自建
的父目录 0700（已存在的目录如 XDG_RUNTIME_DIR 不动）。

帧格式为 JSONL（每行一个 JSON 对象）：

- 请求：`{"id":<i64>,"method":"<str>","params":{…}}`（params 可省略）
- 应答：`{"id":…,"ok":true,"result":…}` 或 `{"id":…,"ok":false,"error":"…"}`
- 事件（`subscribe` 后推入同连接）：`{"event":"popup"|"show_window","params":…}`

| 方法 | params | result |
|------|--------|--------|
| ping | `{}` | `{"version":"…"}` |
| status | `{}` | daemon 状态快照（同 HTTP `/status`） |
| translate | `{"text":"…"}` | Translation（source_lang/target_lang/text/translation/engine） |
| lookup | `{"word":"…"}` | WordCard 词典卡片（未命中为空卡） |
| trigger | `{"source":"selection"\|"clipboard"}` | `{"notified":bool,"translation":…?}`（无可翻文本时 `notified:false` + reason） |
| history | `{"limit":20?}` | HistoryEntry 数组（新→旧） |
| reload | `{}` | `{"restart_required":bool}`：`[daemon]` 字段热生效；引擎字段（default_engine/`[llm]`）需重启 |
| subscribe | `{}` | 订阅事件推送（重复 subscribe 会替换旧订阅） |

事件：`popup`（params 含 translation 与可选 card 词卡）；`show_window`
（params `{"kind":"main"\|"settings"}`，托盘请求开窗）。

注意：协议 JSON 是 GUI 与 daemon 的契约（GUI 有意不依赖 pardon-core crate）；
改路径规则或帧格式需两侧同步（`crates/core/src/ipc.rs` 与
`gui/src-tauri/src/ipc.rs` 各自实现同一约定）。

### 系统托盘

托盘随 pardond 启动（无 D-Bus 时自动降级，daemon 其余功能不受影响）。菜单：

- **打开主窗口 / 设置**：GUI 在运行 → 事件聚焦已有窗口；未运行 → 拉起
  `pardon-gui --main` / `--settings`；
- **复制即翻译**（勾选）：实时开关剪贴板监听并持久化到 config.toml 的
  `auto_translate`（重启 daemon 后仍保持）；
- **退出 pardond**。

### 设置窗口

`pardon-gui --settings`（或主窗口按钮/托盘菜单进入）。可修改 7 个
`[daemon]` 字段（auto_translate / popup / show_word_badge /
copy_translation / notify_timeout_ms / max_text_bytes / dedup_window_ms）
与 `default_engine`。保存即写回 config.toml（toml_edit 原位写，文件里的
注释原样保留）并触发 daemon reload——`[daemon]` 字段热生效；引擎字段改动
提示「重启生效」并提供一键重启按钮。LLM providers 只读展示（id/类型/
model/key 是否已配置）；**api_key / api_key_env 绝不经 GUI 显示或写入**
（写白名单硬编码，不含任何 key 字段）。

### 验收冒烟（手动清单）

1. `pardond` 正常启动，托盘出现图标；`pardon status` 绿。
2. `popup = false` 下按触发快捷键（M2 示例为 Mod+T）→ 桌面通知（M2 行为
   不变）。
3. 设置界面切 `popup = true` → 触发快捷键 → GUI 弹窗（词卡带音标/词形；
   Esc/失焦隐藏；位置记忆）。
4. 主窗口：查 `run` 出词卡 + 译文；查句子出译文；历史列表点击回填再译；
   daemon 未运行时状态栏提示并可一键启动。
5. 托盘：主窗口/设置拉起与聚焦；「复制即翻译」勾选即时生效并持久化
   （重启 daemon 后仍保持）；退出。
6. `pardon history --limit 5` 与 `--json` 输出正确；`pardon translate hello`
   后条目出现。
7. `~/.config/pardon/config.toml` 的注释在 GUI 改动后完好。
8. GUI 在运行时，点托盘『打开主窗口』/『设置』应聚焦/打开对应窗口。

## License

MIT
