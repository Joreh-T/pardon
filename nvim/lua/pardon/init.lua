---@mod pardon.main pardon.nvim — Neovim frontend for the pardon CLI

local M = {
  _cfg = { cli = 'pardon', auto_close = true },
}

---Configure the plugin.
---
---@param opts table? { cli = 'pardon' (executable path), auto_close = true }
function M.setup(opts)
  M._cfg = vim.tbl_deep_extend('force', M._cfg, opts or {})
end

M.async = require('pardon.async')
M.float = require('pardon.float')

-- Delegate entry points; card.lua / trans.lua are placeholders until
-- Task 23 / Task 24 fill them in.
M.lookup = function(word)
  require('pardon.card').show(word)
end
M.translate = function(text, out_mode)
  require('pardon.trans').run(text, out_mode)
end

---Lookup the word under the cursor (or `word` when given).
function M.cursor_lookup(word)
  word = word or vim.fn.expand('<cword>')
  if word ~= '' then
    M.lookup(word)
  end
end

---Translate a range / visual selection / cursor word.
---
---`o` is the user-command table (`{ range, line1, line2 }`; `range == 0`
---means no range was given) or a mapping-made table carrying an explicit
---`text`, or `{ line1, line2, visual = true }`. Falls back to the cursor
---word when nothing else is available.
---@param o table? see above
function M.range_translate(o)
  o = o or {}
  local text = o.text
  if not text and o.line1 and o.line2 and (o.visual or (o.range or 0) > 0) then
    local lines = vim.api.nvim_buf_get_lines(0, o.line1 - 1, o.line2, false)
    text = table.concat(lines, '\n')
  end
  text = text or vim.fn.expand('<cword>')
  if text ~= '' then
    M.translate(text)
  end
end

return M
