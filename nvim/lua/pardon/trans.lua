---@mod pardon.trans Streaming translation (JSONL → float/replace/append/register)
--
-- Consumes `pardon translate --stream --stdin` events (meta → delta* →
-- result) and renders them per the output mode:
--
--   float    growing floatwin: `⟳ 翻译中…` → `→ zh · engine` + translation
--   replace  nvim_buf_set_text over the selection geometry (needs ctx)
--   append   insert the translation after the selection / cursor line
--   register vim.fn.setreg('+', translation)
--
-- Deltas are non-transactional: the float ACCUMULATES them, but the `result`
-- event is authoritative — its `translation` replaces the accumulated text
-- (on mid-stream engine failure the pipeline re-emits the full fallback text
-- as one more delta, so accumulation alone could diverge). replace/append/
-- register apply their effect exactly once, at the result event.
--
-- Errors: the CLI reports them on stderr only (stdout keeps the meta/delta
-- sequence); exit 124 → `超时`, exit 2 → `错误: <stderr first line>` (stderr
-- may be non-JSON, so it is displayed raw).

local async = require('pardon.async')
local float = require('pardon.float')

local M = {
  _done = false, -- test hook: a terminal state was reached (result or error)
  _text = nil, -- test hook: authoritative translation from the result event
  _last_win = nil, -- test hook / singleton float tracking
  _last_buf = nil, -- test hook
  _seq = 0, -- run epoch: stale callbacks from an earlier run never touch hooks
}

local MODES = { float = true, replace = true, append = true, register = true }

---Split a translation into buffer lines (multi-line results become lines).
local function to_lines(text)
  return vim.split(text, '\n', { plain = true })
end

---Translate `text` and render per `mode` (default: setup `default_mode`).
---
---`ctx` carries the selection geometry for replace/append:
---`{ bufnr, start_row, start_col, end_row, end_col }` — 0-based, end col
---exclusive (the same convention as |nvim_buf_set_text()|). Without a ctx,
---replace falls back to float and append inserts after the cursor line.
---
---@param text string text to translate
---@param mode string? float | replace | append | register
---@param ctx table? selection geometry, see above
function M.run(text, mode, ctx)
  local cfg = require('pardon')._cfg
  mode = mode or cfg.default_mode or 'float'
  if not MODES[mode] then
    vim.notify('pardon: unknown translate mode ' .. vim.inspect(mode) .. ', using float', vim.log.levels.WARN)
    mode = 'float'
  end
  if mode == 'replace' and not ctx then
    mode = 'float' -- no selection geometry to replace
  end
  local me = M._seq + 1
  M._seq = me
  M._done, M._text = false, nil
  -- This run applied its result (suppresses the on_exit error path; a late
  -- on_exit from an earlier run is fenced off by the epoch instead).
  local result_applied = false

  local state = { title = '⟳ 翻译中…', body = nil } -- body nil until first delta

  local function lines_now()
    local l = { state.title }
    if state.body ~= nil and state.body ~= '' then
      vim.list_extend(l, to_lines(state.body))
    end
    return l
  end

  -- Float redraw: rewrite the buffer, re-apply the status highlight and
  -- re-fit the window (the text grows from 1 line to the full translation).
  local win, buf
  local function render()
    if not (win and vim.api.nvim_win_is_valid(win) and vim.api.nvim_buf_is_valid(buf)) then
      return
    end
    local lines = lines_now()
    vim.api.nvim_buf_set_lines(buf, 0, -1, false, lines)
    vim.api.nvim_buf_clear_namespace(buf, float.ns, 0, -1)
    vim.api.nvim_buf_add_highlight(buf, float.ns, 'PardonSection', 0, 0, -1)
    local width = 0
    for _, ln in ipairs(lines) do
      width = math.max(width, vim.fn.strdisplaywidth(ln))
    end
    vim.api.nvim_win_set_config(win, {
      relative = 'cursor',
      row = 1,
      col = 0,
      width = math.min(width + 4, 60),
      height = math.max(1, math.min(#lines, vim.o.lines - 4)),
    })
  end

  if mode == 'float' then
    if M._last_win and vim.api.nvim_win_is_valid(M._last_win) then
      float.close(M._last_win)
    end
    local w = float.open({ state.title }, { auto_close = cfg.auto_close })
    win, buf = w.win, w.buf
    M._last_win, M._last_buf = win, buf
  end

  -- Apply the terminal state once. Everything touching the API runs in a
  -- vim.schedule (stdout callbacks are not guaranteed API-safe context).
  local function finish(translation)
    vim.schedule(function()
      if me ~= M._seq then
        return
      end
      local ok, err = pcall(function()
        if mode == 'float' then
          render()
        elseif translation ~= '' then
          local new_lines = to_lines(translation)
          if mode == 'replace' then
            -- Clamp cols to the current line lengths: a `v$` selection ends
            -- one byte past the line (exclusive end), and nvim_buf_set_text,
            -- unlike nvim_buf_get_text, rejects out-of-range columns.
            local span =
              vim.api.nvim_buf_get_lines(ctx.bufnr, ctx.start_row, ctx.end_row + 1, false)
            local sc = math.min(ctx.start_col, #(span[1] or ''))
            local ec = math.min(ctx.end_col, #(span[#span] or ''))
            vim.api.nvim_buf_set_text(ctx.bufnr, ctx.start_row, sc, ctx.end_row, ec, new_lines)
          elseif mode == 'append' then
            -- after the selection's last line, or the cursor line without ctx
            local at = ctx and (ctx.end_row + 1) or vim.fn.line('.')
            vim.api.nvim_buf_set_lines(ctx and ctx.bufnr or 0, at, at, false, new_lines)
          elseif mode == 'register' then
            vim.fn.setreg('+', translation)
          end
        end
      end)
      if not ok then
        vim.notify('pardon: ' .. err, vim.log.levels.ERROR)
      end
      result_applied = true
      M._text = translation
      M._done = true
    end)
  end

  local function on_line(line)
    local ok, ev = pcall(vim.json.decode, line)
    if not ok or type(ev) ~= 'table' then
      return
    end
    if ev.type == 'meta' then
      state.title = '→ ' .. tostring(ev.target or '?') .. ' · ' .. tostring(ev.engine or '?')
      if mode == 'float' then
        vim.schedule(render)
      end
    elseif ev.type == 'delta' then
      state.body = (state.body or '') .. (ev.text or '')
      if mode == 'float' then
        vim.schedule(render)
      end
    elseif ev.type == 'result' then
      -- Authoritative: replace the accumulated deltas with the result text.
      state.body = ev.translation or ''
      if ev.engine and ev.engine ~= '' then
        state.title = '→ ' .. tostring(ev.target or '?') .. ' · ' .. ev.engine
      end
      finish(state.body)
    end
  end

  local function on_exit(obj)
    if result_applied then
      return
    end
    local msg
    if obj.code == 124 then
      msg = '超时'
    elseif obj.code ~= 0 then
      local first = vim.split(obj.stderr or '', '\n', { plain = true })[1] or ''
      first = vim.trim(first)
      msg = first ~= '' and ('错误: ' .. first) or ('错误: exit ' .. obj.code)
    end
    vim.schedule(function()
      if me ~= M._seq then
        return
      end
      if msg then
        if mode == 'float' then
          state.body = msg
          render()
        else
          vim.notify('pardon: ' .. msg, vim.log.levels.ERROR)
        end
      end
      M._done = true
    end)
  end

  async.run(
    { cfg.cli, 'translate', '--stream', '--stdin' },
    { stdin = text },
    on_exit,
    on_line
  )
end

return M
