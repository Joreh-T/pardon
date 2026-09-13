-- Headless test init: put nvim/ on the runtimepath and point the plugin at
-- the mock CLI (tests/mock/pardon), which reproduces the CLI contracts.
local here = vim.fn.fnamemodify(debug.getinfo(1, 'S').source:sub(2), ':h')
vim.opt.runtimepath:append(vim.fn.fnamemodify(here, ':h'))
require('pardon').setup({ cli = here .. '/mock/pardon' })
