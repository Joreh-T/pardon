# pardon.nvim

[pardon](../README.md) 的 Neovim 前端：`:Pardon` 查光标下的词（词卡浮窗），
`:PardonTranslate` 流式翻译选区或光标词（浮窗逐段渲染，或原地替换 /
插入 / 写入寄存器）。

## 要求

- Neovim ≥ 0.10（`vim.system` / `vim.json`）
- pardon CLI 在 `PATH` 上（不在时用 `setup({ cli = … })` 指定，见下）；
  CLI 的安装与词典配置见[仓库 README](../README.md)

## 安装

独立仓库：**<https://github.com/Joreh-T/pardon.nvim>**（源码同源于
[pardon](https://github.com/Joreh-T/pardon) 主仓库的 `nvim/` 目录）。

lazy.nvim / LazyVim（在 `lua/plugins/` 下新建，如 `pardon.lua`）：

```lua
{
  "Joreh-T/pardon.nvim",
  opts = {},
}
```

lazy.nvim 开发模式（克隆在本地、随手改动随 `:Lazy sync` 生效）：

```lua
{
  "Joreh-T/pardon.nvim",
  dev = true,
  opts = {},
}
```

rocks.nvim：

```toml
[plugins."pardon.nvim"]
git = "https://github.com/Joreh-T/pardon.nvim"
```

## 配置

```lua
require('pardon').setup({
  cli = 'pardon',         -- CLI 可执行文件；调试构建可用绝对路径
  auto_close = true,      -- 浮窗在光标下一次移动/进入插入模式时自动关闭
  default_mode = 'float', -- :PardonTranslate 与 <Plug> 映射的默认输出模式
})
```

| 选项 | 默认 | 说明 |
| --- | --- | --- |
| `cli` | `'pardon'` | pardon CLI。不在 `PATH` 或想用调试构建时指定，如 `setup({ cli = '~/pardon/target/debug/pardon' })` |
| `auto_close` | `true` | 词卡 / 翻译浮窗在下一个 `CursorMoved` / `InsertEnter` / `BufLeave` 时自动关闭；`false` 则常驻（`:bd` 关闭） |
| `default_mode` | `'float'` | 翻译默认输出模式：`float` \| `replace` \| `append` \| `register`（见下） |

## 命令

| 命令 | 说明 |
| --- | --- |
| `:Pardon` | 查光标下的词，浮窗显示词卡（音标、词性释义、词形、标签；未命中给相近词） |
| `:PardonTranslate [mode]` | 翻译选区或光标词。带范围时翻译整行（`:'<,'>PardonTranslate`、`:3,5PardonTranslate`）；可选 `mode` 覆盖默认输出模式（支持 tab 补全） |

示例：

```vim
:'<,'>PardonTranslate          " 可视选区（整行），默认模式
:3,5PardonTranslate replace    " 把第 3–5 行替换为译文
:PardonTranslate register      " 光标词翻译进 "+ 寄存器
```

## 键位

插件只注册 `<Plug>` 映射，不设默认键；两种拼法（`<Plug>(Name)` 与
`<Plug>Name`）在 normal / visual 模式都可用。

| `<Plug>` 映射 | 模式 | 说明 |
| --- | --- | --- |
| `<Plug>(PardonLookup)` | `n` / `x` | 光标词 / 选区文本查词卡 |
| `<Plug>(PardonTranslate)` | `n` / `x` | 光标词 / 选区流式翻译（默认模式；visual 精确到字符范围） |

映射示例（注意 `remap = true`，`<Plug>` 需要递归展开）：

```lua
vim.keymap.set('n', '<leader>pw', '<Plug>(PardonLookup)', { remap = true })
vim.keymap.set('x', '<leader>pw', '<Plug>(PardonLookup)', { remap = true })
vim.keymap.set('n', '<leader>pt', '<Plug>(PardonTranslate)', { remap = true })
vim.keymap.set('x', '<leader>pt', '<Plug>(PardonTranslate)', { remap = true })
```

## 输出模式

`mode`（`default_mode`、`:PardonTranslate` 参数或 `range_translate` 的
`o.mode`）决定译文去向：

| mode | 行为 |
| --- | --- |
| `float` | 浮窗流式渲染：首行 `⟳ 翻译中…`，收到 meta 变为 `→ zh · engine`，delta 逐段追加；以 result 事件的 `translation` 为准收尾（权威终态，替换累积文本——引擎中途失败时 CLI 会把兜底全文作为新 delta 发出，累积值可能与终态不同）。超时（exit 124）显示 `超时`；引擎/参数错误（exit 2）显示 `错误: <stderr 首行>`。浮窗单例（新翻译关闭上一个）且随文本自适应尺寸 |
| `replace` | result 后用译文原地替换选区（`nvim_buf_set_text`）。需要选区几何——可视模式 `<Plug>` 映射或带范围的 `:PardonTranslate` 都会携带；光标词没有几何信息，退回 `float` |
| `append` | result 后把译文（可多行）插入选区末行之后；无选区时插在光标行之后 |
| `register` | result 后 `vim.fn.setreg('+', translation)` 写入系统剪贴板寄存器 |

## SSH / 远程环境

选区文本经 stdin 直传 CLI（`pardon translate --stream --stdin`），不读取
也不依赖剪贴板——SSH 到远程机器的终端里，`float` / `replace` / `append`
模式无需任何剪贴板转发（OSC52 等）即可工作。`register` 模式例外：它写
`"+` 寄存器，远程环境需要有可用的 `clipboard` 支持。

## 与全局划词快捷键配合（可选）

桌面快捷键「翻译选区」（`pardon trigger selection`，读 Wayland PRIMARY
选区）对 nvim 有一个已知错位：nvim 的 yank 只写系统剪贴板（CLIPBOARD），
不写 PRIMARY——选区翻译读不到刚 yank 的内容。两种用法任选：

- 在 nvim 里改用「翻译剪贴板」触发（`pardon trigger clipboard`）；
- 或把每次 yank 同步写进 PRIMARY（加入你的 `config/autocmds.lua`）：

```lua
vim.api.nvim_create_autocmd("TextYankPost", {
    desc = "Sync yank to Wayland PRIMARY so selection hotkeys translate it",
    callback = function()
        local content = table.concat(vim.v.event.regcontents or {}, "\n")
        if content ~= "" then
            vim.system({ "wl-copy", "--primary", "--type", "text/plain" }, { stdin = content })
        end
    end,
})
```

## 测试

```bash
bash tests/run_headless.sh   # mock CLI，无需真实词典/网络 → SKELETON_OK + CARD_OK + TRANS_OK
```
