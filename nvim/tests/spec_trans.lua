-- Trans spec: streaming translate — float / replace / append / register modes.
local trans = require('pardon.trans')

-- 1) float mode: mock `translate --stream --stdin` → meta / delta ×2 / result.
--    Deltas accumulate ('跑' + '步'); the result event is authoritative and
--    replaces the accumulated text; the status line shows → target · engine.
trans.run('run', 'float')
vim.wait(3000, function()
  return trans._done
end)
assert(trans._done, 'stream should finish')
assert(trans._text == '跑步', vim.inspect(trans._text))
local lines = vim.api.nvim_buf_get_lines(trans._last_buf, 0, -1, false)
assert(lines[1] == '→ zh · mock', vim.inspect(lines))
assert(vim.tbl_contains(lines, '跑步'), vim.inspect(lines))

-- 2) replace mode with an explicit ctx on a scratch buffer: only the targeted
--    span is replaced, once, at the result event.
local buf = vim.api.nvim_create_buf(false, true)
vim.api.nvim_buf_set_lines(buf, 0, -1, false, { 'i love to run', 'second line' })
trans.run('i love to run', 'replace', {
  bufnr = buf,
  start_row = 0,
  start_col = 0,
  end_row = 0,
  end_col = 13,
})
vim.wait(3000, function()
  return trans._done
end)
assert(trans._done and trans._text == '跑步', 'replace stream should finish')
local rlines = vim.api.nvim_buf_get_lines(buf, 0, -1, false)
assert(rlines[1] == '跑步', vim.inspect(rlines))
assert(rlines[2] == 'second line', vim.inspect(rlines))

-- 3) replace with a `v$`-style ctx (end col one past the last byte): the
--    columns are clamped to the line, not rejected by nvim_buf_set_text.
local vbuf = vim.api.nvim_create_buf(false, true)
vim.api.nvim_buf_set_lines(vbuf, 0, -1, false, { 'i love to run', 'second line' })
trans.run('i love to run', 'replace', {
  bufnr = vbuf,
  start_row = 0,
  start_col = 0,
  end_row = 0,
  end_col = 14, -- v$ leaves the cursor one byte past the line
})
vim.wait(3000, function()
  return trans._done
end)
assert(trans._done, 'v$ replace stream should finish')
local vlines = vim.api.nvim_buf_get_lines(vbuf, 0, -1, false)
assert(vlines[1] == '跑步' and vlines[2] == 'second line', vim.inspect(vlines))

-- 4) append mode with ctx: the translation is inserted after the selection's
--    last line.
local abuf = vim.api.nvim_create_buf(false, true)
vim.api.nvim_buf_set_lines(abuf, 0, -1, false, { 'aaa', 'bbb', 'ccc' })
trans.run('run', 'append', { bufnr = abuf, start_row = 0, start_col = 0, end_row = 1, end_col = 3 })
vim.wait(3000, function()
  return trans._done
end)
assert(trans._done, 'append stream should finish')
local alines = vim.api.nvim_buf_get_lines(abuf, 0, -1, false)
assert(
  vim.inspect(alines) == vim.inspect({ 'aaa', 'bbb', '跑步', 'ccc' }),
  vim.inspect(alines)
)

-- 5) register mode: the translation lands in the "+ register.
trans.run('run', 'register')
vim.wait(3000, function()
  return trans._done
end)
assert(trans._done, 'register stream should finish')
assert(vim.fn.getreg('+') == '跑步', vim.fn.getreg('+'))

-- 6) replace without a ctx falls back to a float.
local prev_win = trans._last_win
trans.run('run', 'replace')
vim.wait(3000, function()
  return trans._done
end)
assert(trans._done and trans._text == '跑步', 'fallback stream should finish')
assert(trans._last_win ~= prev_win, 'replace without ctx should open a float')
assert(vim.api.nvim_win_is_valid(trans._last_win), 'fallback float valid')

-- 7) error path (engine failure, exit 2): stdout stops after meta, the
--    error goes to stderr; the float shows the raw stderr first line and
--    no result ever arrives (_text stays nil).
vim.env.PARDON_MOCK_EXIT = '2'
trans.run('run', 'float')
vim.wait(3000, function()
  return trans._done
end)
vim.env.PARDON_MOCK_EXIT = nil
assert(trans._done, 'error stream should finish')
assert(trans._text == nil, vim.inspect(trans._text))
local elines = vim.api.nvim_buf_get_lines(trans._last_buf, 0, -1, false)
assert(
  vim.tbl_contains(elines, '错误: {"type":"error","code":"mock","message":"mock failure"}'),
  vim.inspect(elines)
)

-- 8) timeout path (exit 124): the float shows 超时.
vim.env.PARDON_MOCK_EXIT = '124'
trans.run('run', 'float')
vim.wait(3000, function()
  return trans._done
end)
vim.env.PARDON_MOCK_EXIT = nil
assert(trans._done, 'timeout stream should finish')
assert(trans._text == nil, vim.inspect(trans._text))
local tlines = vim.api.nvim_buf_get_lines(trans._last_buf, 0, -1, false)
assert(vim.tbl_contains(tlines, '超时'), vim.inspect(tlines))

vim.api.nvim_out_write('TRANS_OK\n')
vim.cmd('qa!')
