---@mod pardon.card Word-card lookup + floatwin rendering

local async = require('pardon.async')
local float = require('pardon.float')

local M = {
  _last_win = nil, -- test hook / singleton tracking
  _last_buf = nil, -- test hook
}

---Title line: `word  /phon/  ●  ★★★  zk·gk·cet4` — exactly two spaces
---between segments; ● only when oxford, ★×collins, tags joined with '·'.
---A CEDICT card (no oxford/collins/tags) renders just `word  /phon/`.
local function title_line(c)
  local s = c.word or ''
  if c.phonetic and c.phonetic.uk then
    s = s .. '  /' .. c.phonetic.uk .. '/'
  end
  if c.oxford then
    s = s .. '  ●'
  end
  -- collins may be 0 (ECDICT star 0): only a positive count gets a star slot,
  -- otherwise the title would end in two decoration spaces.
  if c.collins and c.collins > 0 then
    s = s .. '  ' .. string.rep('★', c.collins)
  end
  if c.tags and #c.tags > 0 then
    s = s .. '  ' .. table.concat(c.tags, '·')
  end
  return s
end

---Render a WordCard as display lines plus per-line highlight groups.
---
---Line order (spec §5.7): title, one line per pos group (`pos  gloss；…`,
---no prefix when pos is empty), `── 词形 ──` + exchange parts (only when any
---word-form exists), `── 释义 ──` + definitions (only when present), and for
---a miss (`found = false`) a single `未命中，相近词: …` line. Every optional
---field may be nil (miss cards carry no phonetic/pos/collins/oxford/tags).
---@param c table decoded WordCard JSON
---@return string[] lines
---@return table[] hl { { lnum, group }, ... }
local function card_lines(c)
  local lines, hl = { title_line(c) }, { { 0, 'PardonWord' } }
  for _, pg in ipairs(c.pos or {}) do
    local prefix = pg.pos ~= '' and (pg.pos .. '  ') or ''
    table.insert(lines, prefix .. table.concat(pg.gloss or {}, '；'))
    table.insert(hl, { #lines - 1, pg.pos ~= '' and 'PardonPos' or 'PardonGloss' })
  end
  local ex = c.exchange
  if ex and (ex.past or ex.pp or ex.ing or ex.third) then
    table.insert(lines, '── 词形 ──')
    table.insert(hl, { #lines - 1, 'PardonSection' })
    local parts = {}
    if ex.past then
      parts[#parts + 1] = 'past: ' .. ex.past
    end
    if ex.pp then
      parts[#parts + 1] = 'pp: ' .. ex.pp
    end
    if ex.ing then
      parts[#parts + 1] = 'ing: ' .. ex.ing
    end
    if ex.third then
      parts[#parts + 1] = '3rd: ' .. ex.third
    end
    table.insert(lines, table.concat(parts, ' · '))
  end
  if c.definition and #c.definition > 0 then
    table.insert(lines, '── 释义 ──')
    table.insert(hl, { #lines - 1, 'PardonSection' })
    for _, d in ipairs(c.definition) do
      table.insert(lines, d)
    end
  end
  if not c.found and c.suggestions and #c.suggestions > 0 then
    table.insert(lines, '未命中，相近词: ' .. table.concat(c.suggestions, ', '))
  end
  return lines, hl
end

---Look up `word` asynchronously and render the card in a floating window.
---
---The card float is a singleton: the previous one is closed before the new
---window opens (an already-auto-closed window id is simply dropped).
---@param word string word to look up
function M.show(word)
  local cfg = require('pardon')._cfg
  async.run({ cfg.cli, 'lookup', word, '--json' }, { text = true }, function(obj)
    -- Exit-1 miss JSON still parses; genuinely non-JSON stdout (e.g. a CLI
    -- usage error) has nothing worth rendering.
    local ok, card = pcall(vim.json.decode, obj.stdout)
    if not ok or type(card) ~= 'table' then
      return
    end
    if M._last_win and vim.api.nvim_win_is_valid(M._last_win) then
      float.close(M._last_win)
    end
    local lines, hl = card_lines(card)
    local w = float.open(lines, { auto_close = cfg.auto_close })
    M._last_win, M._last_buf = w.win, w.buf
    for _, h in ipairs(hl) do
      vim.api.nvim_buf_add_highlight(w.buf, float.ns, h[2], h[1], 0, -1)
    end
  end)
end

return M
