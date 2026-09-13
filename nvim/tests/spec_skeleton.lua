-- Skeleton spec: async runner + mock lookup contract + command/mapping wiring.
local a = require('pardon.async')

-- plugin/pardon.lua must load cleanly and register its surface. (rtp plugin/
-- files auto-load in -l mode too; the guarded runtime is belt-and-suspenders.)
vim.cmd('runtime plugin/pardon.lua')
local cmds = vim.api.nvim_get_commands({})
assert(cmds.Pardon, ':Pardon not registered')
assert(cmds.PardonTranslate, ':PardonTranslate not registered')

local mapped = { n = {}, x = {} }
for _, mode in ipairs({ 'n', 'x' }) do
  for _, m in ipairs(vim.api.nvim_get_keymap(mode)) do
    mapped[mode][m.lhs] = true
  end
end
for _, lhs in ipairs({
  '<Plug>(PardonLookup)',
  '<Plug>PardonLookup',
  '<Plug>(PardonTranslate)',
  '<Plug>PardonTranslate',
}) do
  assert(mapped.n[lhs], lhs .. ' not mapped in normal mode')
  assert(mapped.x[lhs], lhs .. ' not mapped in visual mode')
end

-- Lookup contract via the mock CLI: exit 0 + single-line WordCard JSON.
a.run({ require('pardon')._cfg.cli, 'lookup', 'run', '--json' }, { text = true }, function(obj)
  assert(obj.code == 0, 'exit 0')
  local card = vim.json.decode(obj.stdout)
  assert(card.word == 'run')
  assert(card.phonetic.uk == 'rʌn')
  vim.api.nvim_out_write('SKELETON_OK\n')
  vim.cmd('qa!')
end)
vim.wait(5000, function()
  return false
end)
error('async did not complete')
