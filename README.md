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

## License

MIT
