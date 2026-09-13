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

单词自动走离线词典路由（不联网）；句子走引擎链。默认引擎为有道
（免配置），配置 LLM 后可作为首选引擎：

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
default_engine = "llm"           # llm | youdao | bing（默认 youdao 开箱即用）

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
- `default_engine = "youdao" | "bing"` 时不需要任何配置即可翻译。

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
持流式渲染），`<Plug>` 键位由用户自行映射。安装与配置说明见 `nvim/`
（整理中，随插件一并发布）。

## License

MIT
