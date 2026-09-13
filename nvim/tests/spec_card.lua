-- Card spec: word-card floatwin rendering — hit card, miss card, singleton.
local float = require('pardon.float')
local card = require('pardon.card')

-- Hit card: mock `lookup run --json` → exit 0, full WordCard JSON.
card.show('run')
vim.wait(3000, function()
  return card._last_win ~= nil
end)
assert(card._last_win, 'float should open')
assert(vim.api.nvim_win_is_valid(card._last_win), 'float window valid')
local ok, lines = pcall(vim.api.nvim_buf_get_lines, card._last_buf, 0, -1, false)
assert(ok, 'buffer readable')
assert(vim.tbl_contains(lines, 'run  /rʌn/  ●  ★★★  zk·gk·cet4'), vim.inspect(lines))
assert(vim.tbl_contains(lines, 'n.  跑步'), vim.inspect(lines))
assert(vim.tbl_contains(lines, 'past: ran · pp: run · ing: running · 3rd: runs'), vim.inspect(lines))
-- Per-line highlights were applied in the card namespace (title + 2 pos +
-- 2 section headers).
local marks = vim.api.nvim_buf_get_extmarks(card._last_buf, float.ns, 0, -1, {})
assert(#marks >= 4, 'per-line highlights applied, got ' .. #marks)
local prev_win = card._last_win

-- Miss card: mock `lookup helo --json` → exit 1, found:false + suggestions;
-- the JSON still parses and renders. Singleton: the previous card float must
-- be closed before the new one opens.
card.show('helo')
local changed = vim.wait(3000, function()
  return card._last_win ~= nil and card._last_win ~= prev_win
end)
assert(changed and card._last_win, 'miss float should open')
assert(not vim.api.nvim_win_is_valid(prev_win), 'previous card float closed (singleton)')
local ok2, mlines = pcall(vim.api.nvim_buf_get_lines, card._last_buf, 0, -1, false)
assert(ok2, 'miss buffer readable')
assert(vim.tbl_contains(mlines, 'helo'), vim.inspect(mlines))
assert(vim.tbl_contains(mlines, '未命中，相近词: hello'), vim.inspect(mlines))
local miss_win = card._last_win

-- collins=0 card (ECDICT star 0): zero stars — the title must be exactly
-- `zero  /ˈzɪəroʊ/` with no empty star slot / trailing decoration spaces.
card.show('zero')
local zero_shown = vim.wait(3000, function()
  return card._last_win ~= nil and card._last_win ~= miss_win
end)
assert(zero_shown and card._last_win, 'zero card float should open')
local ok3, zlines = pcall(vim.api.nvim_buf_get_lines, card._last_buf, 0, -1, false)
assert(ok3, 'zero buffer readable')
assert(vim.tbl_contains(zlines, 'zero  /ˈzɪəroʊ/'), vim.inspect(zlines))

vim.api.nvim_out_write('CARD_OK\n')
vim.cmd('qa!')
