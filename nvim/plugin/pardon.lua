if vim.g.loaded_pardon then
  return
end
vim.g.loaded_pardon = 1

local p = require('pardon')

-- Default highlight groups; `default = true` keeps user/colorscheme
-- definitions in charge. A plain link cannot carry the gui attribute
-- (Neovim drops a linked group's own attrs at render time), so PardonWord
-- resolves Identifier once and adds bold.
local id = vim.api.nvim_get_hl(0, { name = 'Identifier', link = false })
vim.api.nvim_set_hl(0, 'PardonWord', vim.tbl_extend('force', id, {
  default = true,
  bold = true,
  cterm = vim.tbl_extend('force', { bold = true }, id.cterm or {}),
}))
for name, link in pairs({
  PardonPhonetic = 'Comment',
  PardonPos = 'Type',
  PardonSection = 'Comment',
  PardonTag = 'Special',
  PardonGloss = 'Normal',
}) do
  vim.api.nvim_set_hl(0, name, { default = true, link = link })
end

-- Text of the active visual selection (marks v .. .), charwise-precise.
local function visual_text()
  local s, e = vim.fn.getpos('v'), vim.fn.getpos('.')
  local srow, scol, erow, ecol = s[2], s[3], e[2], e[3]
  if srow > erow or (srow == erow and scol > ecol) then
    srow, erow, scol, ecol = erow, srow, ecol, scol
  end
  -- nvim_buf_get_text: 0-based rows/cols, end col exclusive.
  return table.concat(
    vim.api.nvim_buf_get_text(0, srow - 1, scol - 1, erow - 1, ecol, {}),
    '\n'
  )
end

vim.api.nvim_create_user_command('Pardon', function()
  p.cursor_lookup()
end, { desc = 'pardon: lookup the word under the cursor' })

vim.api.nvim_create_user_command('PardonTranslate', function(o)
  p.range_translate(o)
end, { range = true, desc = 'pardon: translate range or cursor word' })

-- <Plug> mappings; no default keys are set. Both the canonical
-- "<Plug>(Name)" spelling and the bare "<Plug>Name" alias from the task doc
-- are registered, in normal and visual mode.
local lookup = {
  n = function() p.cursor_lookup() end,
  x = function() p.cursor_lookup(vim.fn.trim(visual_text())) end,
}
local translate = {
  n = function() p.range_translate({}) end,
  x = function() p.range_translate({ text = visual_text(), visual = true }) end,
}
for _, plug in ipairs({ 'PardonLookup', 'PardonTranslate' }) do
  local rhs = plug == 'PardonLookup' and lookup or translate
  for _, mode in ipairs({ 'n', 'x' }) do
    vim.keymap.set(mode, '<Plug>(' .. plug .. ')', rhs[mode], { silent = true })
    vim.keymap.set(mode, '<Plug>' .. plug, rhs[mode], { silent = true })
  end
end
