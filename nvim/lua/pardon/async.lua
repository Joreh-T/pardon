---@mod pardon.async Async subprocess runner (vim.system wrapper)

local M = {}

---Run `argv`, streaming stdout line-by-line.
---
---nvim 0.12 note: when a custom `stdout` handler is attached, the completed
---object's `stdout` field is nil, so this module re-accumulates the full
---output and attaches it to the object handed to `on_exit`.
---
---@param argv string[] command argument vector (argv[1] is the executable)
---@param opts table? extra |vim.system()| opts (e.g. `{ timeout = ms }`);
---   `text = true` is always applied; `stdout` is reserved
---@param on_exit fun(obj: table) completion callback, always re-scheduled
---   onto the main loop; obj = { code, signal, stdout, stderr }
---@param on_stdout_line fun(line: string)? invoked once per complete stdout
---   line (streaming JSONL consumption); a trailing line without `\n` is
---   flushed on exit
---@return vim.SystemObj
function M.run(argv, opts, on_exit, on_stdout_line)
  local sys_opts = vim.tbl_extend('force', { text = true }, opts or {})
  local pending = '' -- trailing (possibly partial) line
  local output = '' -- full stdout, for the completed object
  sys_opts.stdout = function(err, data)
    if err or not data then
      return
    end
    output = output .. data
    pending = pending .. data
    local lines = vim.split(pending, '\n', { plain = true })
    pending = table.remove(lines) -- last element may be incomplete; keep it
    if on_stdout_line then
      for _, l in ipairs(lines) do
        if l ~= '' then
          on_stdout_line(l)
        end
      end
    end
  end
  return vim.system(argv, sys_opts, function(obj)
    if on_stdout_line and pending ~= '' then
      on_stdout_line(pending)
    end
    obj.stdout = output
    -- Move the callback back onto the main loop.
    vim.schedule(function()
      on_exit(obj)
    end)
  end)
end

return M
