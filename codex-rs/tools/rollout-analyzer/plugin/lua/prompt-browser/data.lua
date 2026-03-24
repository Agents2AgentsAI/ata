--- prompt-browser/data.lua
--- Load and parse prompt snapshot + rollout data.

local M = {}

local sessions_dir = vim.fn.expand("~/.ata/sessions")

function M.find_latest_snapshot()
  local cmd = string.format(
    "find %s -name 'prompt-snapshot-*.jsonl' -type f 2>/dev/null | xargs ls -t 2>/dev/null | head -1",
    vim.fn.shellescape(sessions_dir)
  )
  local result = vim.fn.system(cmd):gsub("%s+$", "")
  return result ~= "" and result or nil
end

function M.find_rollout(snapshot_path)
  local dir = vim.fn.fnamemodify(snapshot_path, ":h")
  local session_id = vim.fn.fnamemodify(snapshot_path, ":t"):gsub("^prompt%-snapshot%-", ""):gsub("%.jsonl$", "")
  local files = vim.fn.glob(dir .. "/rollout-*" .. session_id .. ".jsonl", false, true)
  return files[1]
end

function M.parse_snapshots(path)
  local snapshots = {}
  for line in io.lines(path) do
    local ok, obj = pcall(vim.fn.json_decode, line)
    if ok and obj then table.insert(snapshots, obj) end
  end
  return snapshots
end

function M.parse_rollout(path)
  local items = {}
  for line in io.lines(path) do
    local ok, obj = pcall(vim.fn.json_decode, line)
    if not ok or not obj or obj.type ~= "response_item" then goto continue end
    local p = obj.payload or {}
    local it = p.type or ""
    local role = p.role or ""

    if it == "message" then
      local texts = {}
      local content = p.content or {}
      if type(content) == "table" then
        for _, part in ipairs(content) do
          if type(part) == "table" and part.text then table.insert(texts, part.text) end
        end
      elseif type(content) == "string" then
        table.insert(texts, content)
      end
      local chars = 0
      for _, t in ipairs(texts) do chars = chars + #t end
      table.insert(items, {
        kind = "message", role = role, chars = chars, tokens = math.floor(chars / 4),
        texts = texts, name = (texts[1] or ""):sub(1, 50):gsub("\n", " "),
      })
    elseif it == "function_call" then
      local args = p.arguments or ""
      if type(args) == "table" then args = vim.fn.json_encode(args) end
      table.insert(items, {
        kind = "tool_call", name = p.name or "", chars = #args,
        tokens = math.floor(#args / 4), text = args,
      })
    elseif it == "function_call_output" then
      local out = p.output or ""
      if type(out) ~= "string" then out = vim.fn.json_encode(out) end
      table.insert(items, {
        kind = "tool_output", chars = #out, tokens = math.floor(#out / 4),
        text = out, name = string.format("output (%d chars)", #out),
      })
    elseif it == "reasoning" then
      table.insert(items, { kind = "reasoning", chars = 0, tokens = 0, name = "reasoning" })
    end
    ::continue::
  end
  return items
end

function M.categorize(items)
  local cats, order = {}, {}
  for _, item in ipairs(items) do
    local name = item.kind == "message" and item.role or item.kind
    if not cats[name] then
      cats[name] = { name = name, entries = {}, total_tokens = 0 }
      table.insert(order, name)
    end
    table.insert(cats[name].entries, item)
    cats[name].total_tokens = cats[name].total_tokens + (item.tokens or 0)
  end
  local result = {}
  for _, n in ipairs(order) do table.insert(result, cats[n]) end
  return result
end

function M.total_tokens(snapshots)
  if #snapshots == 0 then return 0 end
  local last = snapshots[#snapshots]
  if last.total_prompt_tokens then return last.total_prompt_tokens end
  local base = last.base_instructions_tokens or math.floor((last.base_instructions_chars or 0) / 4)
  local tools = last.tools_tokens or math.floor((last.tools_chars or 0) / 4)
  local inp = last.total_input_tokens or math.floor((last.total_input_chars or 0) / 4)
  return base + tools + inp
end

function M.fmt_tokens(n)
  if n >= 1000 then return string.format("%.1fk", n / 1000) end
  return tostring(n)
end

--- Group items by API call/turn using snapshot item counts.
function M.categorize_by_turn(items, snapshots)
  local cats = {}
  local prev_n = 0
  for i, snap in ipairs(snapshots) do
    local n = snap.input_items_count or 0
    local turn_items = {}
    local turn_tokens = 0
    for j = prev_n + 1, math.min(n, #items) do
      table.insert(turn_items, items[j])
      turn_tokens = turn_tokens + (items[j].tokens or 0)
    end
    if #turn_items > 0 then
      table.insert(cats, {
        name = string.format("Call #%d (%d new)", i, #turn_items),
        entries = turn_items,
        total_tokens = turn_tokens,
      })
    end
    prev_n = n
  end
  return cats
end

return M
