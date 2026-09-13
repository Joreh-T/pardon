---@mod pardon.main pardon.nvim — Neovim frontend for the pardon CLI

local M = {
  _cfg = { cli = 'pardon', auto_close = true, default_mode = 'float' },
}

---Configure the plugin.
---
---@param opts table? {
---   cli = 'pardon' (executable path),
---   auto_close = true,
---   default_mode = 'float' (float | replace | append | register)
--- }
function M.setup(opts)
  M._cfg = vim.tbl_deep_extend('force', M._cfg, opts or {})
end

M.async = require('pardon.async')
M.float = require('pardon.float')

-- Delegate entry points.
M.lookup = function(word)
  require('pardon.card').show(word)
end
M.translate = function(text, out_mode, ctx)
  require('pardon.trans').run(text, out_mode, ctx)
end

---Lookup the word under the cursor (or `word` when given).
function M.cursor_lookup(word)
  word = word or vim.fn.expand('<cword>')
  if word ~= '' then
    M.lookup(word)
  end
end

---Selection geometry of a linewise command range, for replace/append modes:
---0-based rows/cols, end col exclusive (|nvim_buf_set_text()| convention).
local function range_ctx(line1, line2)
  local last = vim.api.nvim_buf_get_lines(0, line2 - 1, line2, false)[1] or ''
  return { bufnr = 0, start_row = line1 - 1, start_col = 0, end_row = line2 - 1, end_col = #last }
end

---First user-command argument as the output mode: `fargs` is the split list
---for `:command` invocations; a table `args` is accepted for programmatic
---callers. (A command's `args` field is the RAW string, not a list.)
local function arg_mode(o)
  if type(o.fargs) == 'table' then
    return o.fargs[1]
  end
  if type(o.args) == 'table' then
    return o.args[1]
  end
  return nil
end

---Translate a range / visual selection / cursor word.
---
---`o` is the user-command table (`{ range, line1, line2, fargs }`; `range == 0`
---means no range was given; `fargs[1]` is the optional output mode accepted by
---`:PardonTranslate [mode]`) or a mapping-made table carrying an explicit
---`text` (plus optionally its selection geometry in `ctx`), or
---`{ line1, line2, visual = true }`. Mode resolution: `o.mode` →
---`o.fargs[1]` (or a table `o.args[1]`) → setup `default_mode`. Falls back to
---the cursor word when no other text source is available.
---@param o table? see above
function M.range_translate(o)
  o = o or {}
  local text, ctx = o.text, o.ctx
  if not text and o.line1 and o.line2 and (o.visual or (o.range or 0) > 0) then
    local lines = vim.api.nvim_buf_get_lines(0, o.line1 - 1, o.line2, false)
    text = table.concat(lines, '\n')
    ctx = ctx or range_ctx(o.line1, o.line2)
  end
  text = text or vim.fn.expand('<cword>')
  if text == '' then
    return
  end
  local mode = o.mode or arg_mode(o) or M._cfg.default_mode
  M.translate(text, mode, ctx)
end

return M
