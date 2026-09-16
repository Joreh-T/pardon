# pardon

> **English** — pardon is a Chinese↔English translation tool for Linux/Wayland:
> an offline dictionary (3.4M entries) for instant word cards, LLM-powered
> sentence translation with a free Google fallback, hotkey text-selection
> popups, a Tauri GUI with themes, and deep nvim integration. Daemon + thin
> clients: the CLI keeps working even when the daemon is down.

Pardon my French —— 一个为 Linux/Wayland 桌面与终端打造的中英互译工具。

**特性一览**

- **离线词卡**：ECDICT 340 万词条 + CC-CEDICT，划词即出音标、词性释义、
  词形变化、柯林斯星级——微秒级、不联网
- **LLM 句子翻译**：任意 OpenAI 兼容端点 / Anthropic / Ollama，可自定义
  提示词；免费 Google 端点作兜底，多引擎自动降级
- **划词即译**：`Mod+Y` 翻译鼠标选区、`Mod+Shift+Y` 翻译剪贴板，结果以
  GUI 弹窗或桌面通知展示（词卡风格分层）
- **Tauri 2 GUI**：主窗口（查词/翻译/历史）、5 套主题（含跟随系统）、系统托盘、设置界面（LLM provider 增删改、api_key 写入式
  不回显、配置热生效）
- **翻译历史**：本地 SQLite，上限 1000 条，`pardon history` 随查
- **nvim 深度集成**：`:Pardon` 光标词浮窗词卡、`:PardonTranslate` 选区
  流式翻译（float / 原地替换 / 插入 / 写寄存器），SSH 远程可用
- **发音**：在线音频缓存 + espeak-ng 离线兜底

**平台要求**：Linux + Wayland（X11/Windows 见 [Roadmap](#roadmap)）；
Rust stable（建议 ≥ 1.78）构建；构建 GUI 另需 Node ≥ 18 + npm。

## 架构

Daemon + 瘦客户端：常驻的 `pardond` 承担「必须在场」的能力（剪贴板监听、
IPC、托盘、事件推送），`pardon` CLI 内嵌同一核心直查词典（daemon 不在也
完全可用），GUI 是纯 IPC 客户端（可替换）。

```mermaid
flowchart TB
    subgraph clients["客户端（可替换入口）"]
        CLI["pardon CLI\n毫秒级·内嵌核心"]
        NVIM["pardon.nvim\n调 CLI"]
        GUI["pardon-gui (Tauri 2)\n纯 IPC 客户端"]
    end
    subgraph daemon["pardond（常驻 daemon）"]
        CLIP["剪贴板监听桥"] --> H["处理链：防回环→分流→输出分发"]
        UDS["UDS IPC"] --> H
        HTTP["HTTP 触发口"] --> H
        TRAY["托盘"]
    end
    CORE["pardon-core（静态内嵌于 CLI 与 daemon）\n分流器 · 词典 · 引擎链 · TTS · 配置 · 历史"]
    DICT[("词典 SQLite")]
    LLM["LLM API / Google"]

    CLI -.静态内嵌.-> CORE
    H --> CORE
    CORE --> DICT
    CORE --> LLM
    GUI <--> UDS
```

可编辑的详细图纸（excalidraw，可在 [excalidraw.com](https://excalidraw.com)
打开）：[系统架构图](docs/architecture.excalidraw) ·
[翻译执行流程图](docs/flow.excalidraw)。

## 安装

系统依赖（Debian/Ubuntu；其他发行版装等价包）：

```bash
# 运行依赖：wl-clipboard（剪贴板）、桌面通知服务（mako/dunst/noctalia 任一）
# 构建依赖：pkg-config、ALSA 头（TTS 播放 rodio）、构建 GUI 还需 webkit2gtk
sudo apt install wl-clipboard pkg-config libasound2-dev \
     libwebkit2gtk-4.1-dev build-essential libxdo-dev librsvg2-dev libssl-dev
```

三件套（可只装 CLI，其余按需）：

```bash
git clone https://github.com/Joreh-T/pardon && cd pardon

cargo install --path crates/cli --bin pardon       # CLI（查词/翻译/发音/触发）
cargo install --path crates/daemon --bin pardond   # 常驻 daemon（划词/通知/托盘）
# GUI：先构建前端再装（顺序不能反——tauri 编译期嵌入前端产物）
cd gui && npm install && npm run build && cd src-tauri \
  && cargo install --path . --bin pardon-gui && cd ../..
```

词典数据不入仓库（体积与许可原因），导入一次即可，见
**[dicts/README.md](dicts/README.md)**——里面还有几秒走通全链路的 mini
fixture 冒烟一节。

## 快速上手

### 1. 查词（离线）

```bash
$ pardon lookup run --json
{"found":true,"word":"run","phonetic":{"uk":"rʌn"},"pos":[{"pos":"n.","gloss":["跑步"]},{"pos":"v.","gloss":["跑","运转"]}],"definition":["move fast","operate"],"exchange":{"past":"ran","pp":"run","ing":"running","third":"runs","lemma":"run"},"collins":3,"oxford":true,"tags":["zk","gk","cet4"],"source":"ecdict"}

$ pardon lookup 你好 --json
{"found":true,"word":"你好","phonetic":{"uk":"nǐ hao"},"pos":[{"pos":"","gloss":["you (informal)","hello"]}],"oxford":false,"source":"cedict"}
```

中文词走 CEDICT（带拼音，数字声调自动转声调符号），英文词走 ECDICT
（音标、词性分组释义、词形变化、Collins/Oxford 标签）。

### 2. 翻译（引擎链）

单词自动走离线词典路由（不联网）；句子走引擎链（LLM → Google → 有道
→ 必应，自动降级）。Google 走非官方免费端点（Chrome 词典扩展同款，
无需配置），可能失效或限流；有道/Bing 的免费端点已于 2026-09 失效
（仅作殿后兜底）。要稳定的句子翻译质量请配置 LLM provider（示例见下方
[配置](#配置)节）；查词（离线词典）与发音不受影响。

```bash
$ echo "The quick brown fox jumps over the lazy dog." | pardon translate --stdin --stream
{"type":"meta","source":"en","target":"zh","engine":"auto"}
{"type":"delta","text":"…"}            # 流式增量（LLM 引擎时逐段输出）
{"type":"result","source":"en","target":"zh","engine":"…","text":"…","translation":"…"}
```

`--stream` 逐行输出 JSONL 事件（`meta` → `delta`… → `result`）；不加
`--stream` 可用 `--json` 拿单行结果 JSON。常用参数：

- `--engine llm|google|youdao|bing|auto`：指定引擎（指定后不做降级链）；
  `auto` 会按语言自动选择并允许失败降级到下一个引擎；
  `--engine google` 显式走 Google 免费端点。
- `--source` / `--target en|zh|auto`：仅在显式指定 `--engine` 时生效；
  `--engine auto` 模式下方向由文本自动检测。
- `--timeout N`：整次操作超时秒数（默认 30；超时输出
  `{"type":"error","code":"timeout",…}` 并以退出码 124 结束）。
- 翻译一个单词（如 `pardon translate run`）优先离线词典卡片。

### 3. 朗读

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
default_engine = "llm"           # llm | google | youdao | bing（youdao/bing 免费端点已失效，google 为非官方免费端点，句子翻译建议 llm）

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
- `default_engine = "google"` 时不需要任何配置（默认兜底链第一位即
  Google），但走非官方免费端点，可能失效或限流。
- `default_engine = "youdao" | "bing"` 时不需要任何配置，但其免费端点
  已于 2026-09 失效，句子翻译建议配置 LLM。

以上 provider 与引擎项也可以全部在 GUI 设置界面里管理（见
[设置窗口](#设置窗口)），无需手改文件。

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
| `PARDON_HTTP_BIND` | HTTP 触发口绑定地址覆盖（仅回环） |
| `PARDON_IPC_SOCK` | UDS 套接字路径覆盖（调试用） |

## nvim 集成

nvim 插件独立发布于 [Joreh-T/pardon.nvim](https://github.com/Joreh-T/pardon.nvim)，
并以 git submodule 挂载在本仓库 `nvim/` 路径（代码实体只在插件仓库——克隆
主仓库时加 `--recurse-submodules` 可一并取得）：`:Pardon` 查光标下的词、
`:PardonTranslate` 翻译选区或光标词，词卡与译文在浮动窗口展示（翻译支
持流式渲染），`<Plug>` 键位由用户自行映射。安装与配置说明见
[nvim/README.md](nvim/README.md)。

## 后台常驻与划词翻译（pardond）

常驻 daemon 监听剪贴板（可选「复制即翻译」），快捷键经 `pardon trigger`
唤起翻译并弹 GUI 弹窗或桌面通知。`pardon lookup/translate/speak` 不依赖
daemon（内嵌直查）。

### 运行（推荐：systemd user service）

    mkdir -p ~/.config/systemd/user
    cp dist/systemd/pardon.service ~/.config/systemd/user/
    systemctl --user daemon-reload && systemctl --user enable --now pardon

无 systemd / 桌面自启场景：`pardon daemon start`（日志
`~/.local/share/pardon/log/pardond.log`）/ `pardon daemon stop`。
niri 用户也可以 `spawn-sh-at-startup "pardon daemon start"`。

依赖：`wl-clipboard`（wl-paste/wl-copy，剪贴板监听与读写）；通知服务
（mako/dunst 等）。缺失时 daemon 自动降级为纯 HTTP 触发口模式
（`pardon status` 可见）。

### 快捷键（niri 示例，键位自定）

```kdl
binds {
    Mod+Y { spawn "pardon" "trigger" "selection"; }     // 翻译鼠标选区（PRIMARY）
    Mod+Shift+Y { spawn "pardon" "trigger" "clipboard"; } // 翻译剪贴板
    Mod+G { spawn "pardon-gui"; }                       // 唤起/聚焦 GUI（单实例）
}
```

（GNOME 等桌面：系统设置 → 自定义快捷键 → 命令 `pardon trigger selection`。）

> 小贴士：终端里的 vim/nvim 等 TUI 应用不会把 yank 写进 PRIMARY 选区——
> 在这些应用里用「剪贴板」触发更符合直觉，或参考 nvim/README.md 的
> TextYankPost 方案。

### HTTP 触发口（localhost only）

    curl -s http://127.0.0.1:7377/status
    curl -s -X POST http://127.0.0.1:7377/translate -H 'content-type: application/json' -d '{"text":"hello world"}'
    curl -s http://127.0.0.1:7377/trigger/selection

JSON 说明：响应中的可选项在缺省时直接省略键（如 trigger 响应未产生译文时
没有 `translation` 键），而非输出显式 `null`——消费方应把缺失键当作 `null`。

安全说明：只绑定回环地址、无鉴权——本地任意进程都可触发/关停（用户级信任域，
与剪贴板本身同权限级）。UDS 套接字同理，靠文件权限（0600）限定本用户。
改端口见下方配置。

### [daemon] 配置（~/.config/pardon/config.toml，均可省略）

| 键 | 默认 | 说明 |
|----|------|------|
| http_bind | "127.0.0.1:7377" | HTTP 触发口（仅回环地址；PARDON_HTTP_BIND 环境变量可覆盖） |
| auto_translate | true | 复制即翻译总开关 |
| max_text_bytes | 5120 | 自动翻译文本上限（字节），超过忽略 |
| dedup_window_ms | 10000 | 同内容去重时间窗（防回环） |
| copy_translation | false | 译文自动写回剪贴板（写回前登记防回环，不会 ping-pong）；翻译进行期间的其他复制会被译文覆盖（防回环只防循环，不防丢失） |
| notify_timeout_ms | 5000 | 通知显示时长 |
| show_word_badge | false | 词卡显示学习徽章行（柯林斯星级 · 牛津核心 · 考试标签） |
| popup | false | 翻译结果优先 GUI 弹窗展示（无 GUI 订阅时回退桌面通知，见[弹窗模式](#弹窗模式popup)） |

## 图形界面（pardon-gui）

Tauri 2 GUI（`gui/` 目录，独立 cargo 项目）：快捷键弹窗、主窗口（查词/
翻译/历史）、系统托盘、设置界面。GUI 是纯 IPC 客户端——所有翻译/词典/
历史数据经下方 UDS 协议向 pardond 请求，自身不内嵌翻译逻辑（可替换
客户端）。GUI 未检测到 daemon 时会自动拉起；主窗口关闭后进程由隐藏的
弹窗窗口保活，再按快捷键即重新唤出主窗口。

### 构建与运行

见 [安装](#安装) 一节的 GUI 部分（先 `npm run build` 再 cargo）。
运行 `pardon-gui`（加 `--settings` 启动即开设置窗口）。

### 桌面图标（应用启动器）

让 pardon 出现在应用启动器（GNOME/noctalia 等）并正确关联窗口图标：

```bash
# 图标（512px，与托盘同款）
mkdir -p ~/.local/share/icons/hicolor/512x512/apps
cp assets/pardon-512.png ~/.local/share/icons/hicolor/512x512/apps/pardon.png
# 桌面项（Exec 换成绝对路径更稳，启动器不一定继承 ~/.cargo/bin）
sed 's|^Exec=pardon-gui$|Exec='"$HOME"'/.cargo/bin/pardon-gui|' \
    dist/desktop/pardon-gui.desktop > ~/.local/share/applications/pardon-gui.desktop
```

Wayland app_id 为 `pardon-gui`，与桌面文件名一致，启动器图标自动关联窗口。

### 弹窗模式（popup）

`[daemon] popup = true`（默认 false，通知模式行为不变；也可在设置界面切换）。
开启后翻译结果改用 GUI 弹窗展示：词卡带音标/词形，内容高度自适应
（宽固定 420），Esc 或失焦隐藏；隐藏时的位置记忆在
`~/.local/share/pardon/gui-state.json`，下次在原位弹出。无 GUI 订阅时自动
回退桌面通知，GUI 退出/断开即恢复通知——两条通路永不低于纯通知模式的
可用性。

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

### 设置窗口

`pardon-gui --settings`（或主窗口按钮/托盘菜单进入）。可修改 7 个
`[daemon]` 字段（auto_translate / popup / show_word_badge /
copy_translation / notify_timeout_ms / max_text_bytes / dedup_window_ms）、
`default_engine` 与「默认 LLM provider」下拉。保存即写回 config.toml
（toml_edit 原位写，文件里的注释原样保留）并触发 daemon reload——
`[daemon]` 字段热生效；引擎相关项（default_engine / default_provider /
provider 条目）改动后出现「重启 pardond」横幅按钮，一键重启。

**LLM provider 管理**：providers 列表直接增/删/改（id / type /
base_url / model）。守卫：删除正被 `llm.default_provider` 指向的
provider 会被拒绝（先切走默认再删）；`default_engine = "llm"` 时必须
选定一个有效 provider，悬空指向在保存前拦截；编辑改 id 时若旧 id 是
默认指向，`default_provider` 同步改写。

**api_key 政策（写入式）**：GUI 永不回显已存 key——读取侧脱敏，每个
provider 只带 `has_api_key` 存在性标志；表单里的 key 框是密码框，留空＝
保留现值，输入新值才覆盖（明文写入 config.toml 的 `api_key`，风险同
[配置](#配置)节所述）；「清除已存 key」是编辑表单里的独立按钮。
`api_key_env` 不经 GUI（读侧同样脱敏，配置里已有的原样保留）。key 只
存于 config.toml。

**主题**：「界面主题」下拉——跟随系统（prefers-color-scheme）/
Catppuccin 摩卡 / Catppuccin 拿铁 / macOS 浅色 / Nord。选择即时生效并
跨窗口（主窗口/弹窗/设置）同步。主题偏好存 GUI 本地（webview
localStorage），不写 config.toml。

### 系统托盘

托盘随 pardond 启动（无 D-Bus 时自动降级，daemon 其余功能不受影响）。菜单：

- **打开主窗口 / 设置**：GUI 在运行 → 事件聚焦已有窗口；未运行 → 拉起
  `pardon-gui --main` / `--settings`；
- **复制即翻译**（勾选）：实时开关剪贴板监听并持久化到 config.toml 的
  `auto_translate`（重启 daemon 后仍保持）；
- **退出 pardond**。

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

## Roadmap

- **M4 — Windows**：clipboard-win、RegisterHotKey、named-pipe IPC、SAPI TTS、
  安装包（架构已按平台 trait 抽象预留）
- **M5 — 生态（可选）**：StarDict 用户词库、PDF 断句清洗、更多语种、生词本
- X11 支持视需求评估（当前 Wayland 优先）

## 开发

```bash
# Rust workspace（core/cli/platform/daemon）
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check

# GUI（独立 cargo 项目）
cd gui && npm run check && npm run test && npm run build
cd src-tauri && cargo test && cargo clippy --all-targets -- -D warnings
```

注意：GUI 构建顺序是「先 `npm run build` 产出 `dist/` 再 cargo」——tauri
在编译期嵌入前端产物，`dist/` 缺失会 panic。

### 插件开发（nvim/，submodule）

插件源码实体在 [Joreh-T/pardon.nvim](https://github.com/Joreh-T/pardon.nvim)，
本仓库以 submodule 引用。改动插件的流程（两步都不能少，漏第二步会产生断指针）：

```bash
cd nvim && git add -A && git commit -m "…" && git push   # ① 子模块仓库提交并推送
cd .. && git add nvim && git commit -m "chore: bump pardon.nvim" && git push  # ② 主仓库记指针并推送
```

## 数据与致谢

- [ECDICT](https://github.com/skywind3000/ECDICT)（英→中主词典）与
  [CC-CEDICT](https://www.mdbg.net/chinese/dictionary?page=cedict)（中→英）
  的数据**不在本仓库内**，由用户自行下载导入（见
  [dicts/README.md](dicts/README.md)）；两份词典的许可归其上游项目所有
  （CC-CEDICT 为 CC BY-SA 4.0）。
- 灵感与参考：pot-desktop、EasyDict、CopyTranslator、translate.nvim 等项目。

## License

MIT
