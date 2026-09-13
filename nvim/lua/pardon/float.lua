---@mod pardon.float Generic floating window: adaptive size, auto-close

local M = {}
M.ns = vim.api.nvim_create_namespace('pardon')

---Open a cursor-relative float showing `lines`.
---
---Width = min(longest display width + 4, 60); height = #lines capped by the
---window height. Unless `opts.auto_close == false`, the float closes itself
---once on the next CursorMoved/InsertEnter/BufLeave.
---
---@param lines string[] buffer lines
---@param opts table? {
---   relative = 'cursor', row = 1, col = 0, auto_close = true }
---@return table { win = winid, buf = bufnr }
function M.open(lines, opts)
  opts = opts or {}
  local width = 0
  for _, l in ipairs(lines) do
    width = math.max(width, vim.fn.strdisplaywidth(l))
  end
  width = math.min(width + 4, 60)
  local height = math.max(1, math.min(#lines, vim.o.lines - 4))
  local buf = vim.api.nvim_create_buf(false, true)
  vim.api.nvim_buf_set_lines(buf, 0, -1, false, lines)
  local win = vim.api.nvim_open_win(buf, false, {
    relative = opts.relative or 'cursor',
    row = opts.row or 1,
    col = opts.col or 0,
    width = width,
    height = height,
    style = 'minimal',
    border = 'rounded',
    focusable = false,
    zindex = 50,
  })
  if opts.auto_close ~= false then
    vim.api.nvim_create_autocmd(
      { 'CursorMoved', 'CursorMovedI', 'InsertEnter', 'BufLeave' },
      { once = true, callback = function() M.close(win) end }
    )
  end
  return { win = win, buf = buf }
end

---Close `win` and wipe its buffer if still valid.
function M.close(win)
  if vim.api.nvim_win_is_valid(win) then
    local buf = vim.api.nvim_win_get_buf(win)
    vim.api.nvim_win_close(win, true)
    if vim.api.nvim_buf_is_valid(buf) then
      vim.api.nvim_buf_delete(buf, { force = true })
    end
  end
end

return M
