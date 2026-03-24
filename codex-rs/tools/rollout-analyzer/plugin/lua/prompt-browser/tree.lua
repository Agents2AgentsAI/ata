--- prompt-browser/tree.lua
--- Sidebar tree + preview pane, matching prompt-inspector style.

local M = {}
local data_mod = require("prompt-browser.data")

M._buf = nil
M._win = nil
M._preview_buf = nil
M._preview_win = nil
M._data = nil          -- { items, snapshots, total_tokens, ... }
M._categories = nil    -- current computed categories
M._expanded = {}
M._line_map = {}
M._show_tokens = true
M._sort_mode = "position" -- "position" | "tokens"
M._group_mode = "flat"    -- "flat" | "role" | "turn"

-- ── Highlights ──────────────────────────────────────────────

local function define_highlights()
  local links = {
    PromptBrowserCategory = "Title",
    PromptBrowserEntry = "Normal",
    PromptBrowserTokens = "Comment",
    PromptBrowserFooter = "Comment",
    PromptBrowserDeveloper = "Type",
    PromptBrowserUser = "String",
    PromptBrowserAssistant = "Function",
    PromptBrowserToolCall = "Type",
    PromptBrowserToolOutput = "DiagnosticInfo",
    PromptBrowserReasoning = "Comment",
  }
  for group, target in pairs(links) do
    vim.api.nvim_set_hl(0, group, { link = target, default = true })
  end
end

-- ── Render ──────────────────────────────────────────────────

local function sort_entries(entries, mode)
  if mode == "tokens" then
    local sorted = vim.list_extend({}, entries)
    table.sort(sorted, function(a, b) return (a.tokens or 0) > (b.tokens or 0) end)
    return sorted
  end
  return entries
end

local function sort_categories(cats, mode)
  if mode == "tokens" then
    local sorted = vim.list_extend({}, cats)
    table.sort(sorted, function(a, b) return (a.total_tokens or 0) > (b.total_tokens or 0) end)
    return sorted
  end
  return cats
end

function M._regroup()
  if M._group_mode == "turn" then
    M._categories = data_mod.categorize_by_turn(M._data.items or {}, M._data.snapshots or {})
  elseif M._group_mode == "role" then
    M._categories = data_mod.categorize(M._data.items or {})
  else -- flat
    M._categories = nil
  end
end

function M._render()
  if not M._buf or not vim.api.nvim_buf_is_valid(M._buf) then return end

  local lines = {}
  local highlights = {}
  M._line_map = {}

  -- Flat mode: render items directly without category headers
  if M._group_mode == "flat" then
    local items = M._data.items or {}
    if M._sort_mode == "tokens" then
      items = sort_entries(items, "tokens")
    end
    for _, entry in ipairs(items) do
      local ename = (entry.name or entry.kind or "?"):sub(1, 40)
      local entry_line
      if M._show_tokens then
        entry_line = string.format("\u{25cf} %s  %st", ename, data_mod.fmt_tokens(entry.tokens or 0))
      else
        entry_line = string.format("\u{25cf} %s", ename)
      end
      table.insert(lines, entry_line)
      local eidx = #lines
      M._line_map[eidx] = { type = "entry", entry = entry }

      local hl = "PromptBrowserEntry"
      if entry.kind == "message" then
        if entry.role == "developer" then hl = "PromptBrowserDeveloper"
        elseif entry.role == "user" then hl = "PromptBrowserUser"
        elseif entry.role == "assistant" then hl = "PromptBrowserAssistant" end
      elseif entry.kind == "tool_call" then hl = "PromptBrowserToolCall"
      elseif entry.kind == "tool_output" then hl = "PromptBrowserToolOutput"
      elseif entry.kind == "reasoning" then hl = "PromptBrowserReasoning" end
      table.insert(highlights, { eidx - 1, hl, 0, -1 })

      if M._show_tokens then
        local tok_str = data_mod.fmt_tokens(entry.tokens or 0) .. "t"
        local col_end = #entry_line
        local col_start = col_end - #tok_str
        if col_start >= 0 then
          table.insert(highlights, { eidx - 1, "PromptBrowserTokens", col_start, col_end })
        end
      end
    end
  else

  local categories = sort_categories(M._categories or {}, M._sort_mode)

  for _, cat in ipairs(categories) do
    local expanded = M._expanded[cat.name] or false
    local entries = sort_entries(cat.entries or {}, M._sort_mode)
    local arrow = expanded and "\u{25bc}" or "\u{25ba}"

    -- Category line
    local cat_line
    if M._show_tokens then
      cat_line = string.format("%s %s  %st  (%d)", arrow, cat.name, data_mod.fmt_tokens(cat.total_tokens), #entries)
    else
      cat_line = string.format("%s %s  (%d)", arrow, cat.name, #entries)
    end
    table.insert(lines, cat_line)
    local li = #lines
    M._line_map[li] = { type = "category", name = cat.name }
    table.insert(highlights, { li - 1, "PromptBrowserCategory", 0, -1 })

    if expanded then
      for _, entry in ipairs(entries) do
        local ename = (entry.name or entry.kind or "?"):sub(1, 40)
        local entry_line
        if M._show_tokens then
          entry_line = string.format("  \u{25cf} %s  %st", ename, data_mod.fmt_tokens(entry.tokens or 0))
        else
          entry_line = string.format("  \u{25cf} %s", ename)
        end
        table.insert(lines, entry_line)
        local eidx = #lines
        M._line_map[eidx] = { type = "entry", entry = entry, category = cat.name }

        -- Color by type
        local hl = "PromptBrowserEntry"
        if entry.kind == "message" then
          if entry.role == "developer" then hl = "PromptBrowserDeveloper"
          elseif entry.role == "user" then hl = "PromptBrowserUser"
          elseif entry.role == "assistant" then hl = "PromptBrowserAssistant" end
        elseif entry.kind == "tool_call" then hl = "PromptBrowserToolCall"
        elseif entry.kind == "tool_output" then hl = "PromptBrowserToolOutput"
        elseif entry.kind == "reasoning" then hl = "PromptBrowserReasoning" end
        table.insert(highlights, { eidx - 1, hl, 0, -1 })

        -- Dim token count
        if M._show_tokens then
          local tok_str = data_mod.fmt_tokens(entry.tokens or 0) .. "t"
          local col_end = #entry_line
          local col_start = col_end - #tok_str
          if col_start >= 0 then
            table.insert(highlights, { eidx - 1, "PromptBrowserTokens", col_start, col_end })
          end
        end
      end
    end
  end

  end -- if flat / else categories

  -- Footer
  table.insert(lines, "")
  local total = M._data.total_tokens or 0
  local footer = string.format("~%s tokens  %d calls", data_mod.fmt_tokens(total), M._data.snapshot_count or 0)
  table.insert(lines, footer)
  table.insert(highlights, { #lines - 1, "PromptBrowserFooter", 0, -1 })
  table.insert(lines, string.format("Group: %s  Sort: %s | g=group s=sort T=tok", M._group_mode, M._sort_mode))
  table.insert(highlights, { #lines - 1, "PromptBrowserFooter", 0, -1 })

  vim.api.nvim_set_option_value("modifiable", true, { buf = M._buf })
  vim.api.nvim_buf_set_lines(M._buf, 0, -1, false, lines)
  vim.api.nvim_set_option_value("modifiable", false, { buf = M._buf })

  local ns = vim.api.nvim_create_namespace("prompt_browser_tree")
  vim.api.nvim_buf_clear_namespace(M._buf, ns, 0, -1)
  for _, hl in ipairs(highlights) do
    vim.api.nvim_buf_add_highlight(M._buf, ns, hl[2], hl[1], hl[3], hl[4])
  end
end

-- ── Preview ─────────────────────────────────────────────────

local function prettify_json(text)
  local trimmed = text:match("^%s*(.-)%s*$")
  if trimmed:sub(1, 1) == "{" or trimmed:sub(1, 1) == "[" then
    local ok, obj = pcall(vim.fn.json_decode, trimmed)
    if ok then
      return vim.fn.json_encode(obj) -- vim encodes with indent when using split
      -- Actually use vim.inspect or manual pretty print
    end
  end
  return nil
end

local function show_preview(entry)
  if not entry then return end

  -- Create preview buffer if needed
  if not M._preview_buf or not vim.api.nvim_buf_is_valid(M._preview_buf) then
    M._preview_buf = vim.api.nvim_create_buf(false, true)
    vim.api.nvim_set_option_value("buftype", "nofile", { buf = M._preview_buf })
    vim.api.nvim_set_option_value("buflisted", false, { buf = M._preview_buf })
    vim.api.nvim_set_option_value("swapfile", false, { buf = M._preview_buf })
  end

  -- Ensure preview window exists
  if not M._preview_win or not vim.api.nvim_win_is_valid(M._preview_win) then
    for _, w in ipairs(vim.api.nvim_tabpage_list_wins(0)) do
      if w ~= M._win then
        M._preview_win = w
        break
      end
    end
    if not M._preview_win or not vim.api.nvim_win_is_valid(M._preview_win) then
      vim.cmd("wincmd l")
      if vim.api.nvim_get_current_win() == M._win then
        vim.cmd("botright vsplit")
      end
      M._preview_win = vim.api.nvim_get_current_win()
      vim.api.nvim_set_current_win(M._win)
    end
  end

  vim.api.nvim_win_set_buf(M._preview_win, M._preview_buf)

  -- Build content lines
  local lines = {}
  local kind = entry.kind

  if kind == "message" then
    table.insert(lines, string.format("%s  %d chars  ~%s tok  %d parts",
      entry.role:upper(), entry.chars, data_mod.fmt_tokens(entry.tokens), #(entry.texts or {})))
    table.insert(lines, "")
    for i, text in ipairs(entry.texts or {}) do
      if #(entry.texts or {}) > 1 then
        table.insert(lines, string.format("── part %d/%d (%d chars) ──", i, #entry.texts, #text))
        table.insert(lines, "")
      end
      for _, l in ipairs(vim.split(text, "\n", { plain = true })) do
        table.insert(lines, l)
      end
      table.insert(lines, "")
    end
  elseif kind == "tool_call" then
    table.insert(lines, string.format("CALL  %s  ~%s tok", entry.name, data_mod.fmt_tokens(entry.tokens)))
    table.insert(lines, "")
    local text = entry.text or ""
    -- Pretty-print JSON args
    local ok, obj = pcall(vim.fn.json_decode, text)
    if ok then
      for _, l in ipairs(vim.split(vim.fn.json_encode(obj), "\n", { plain = true })) do
        table.insert(lines, l)
      end
    else
      for _, l in ipairs(vim.split(text, "\n", { plain = true })) do
        table.insert(lines, l)
      end
    end
  elseif kind == "tool_output" then
    table.insert(lines, string.format("OUTPUT  %d chars  ~%s tok", entry.chars, data_mod.fmt_tokens(entry.tokens)))
    table.insert(lines, "")
    local text = entry.text or ""
    -- Pretty-print JSON outputs
    local trimmed = text:match("^%s*(.-)%s*$")
    if trimmed:sub(1, 1) == "{" or trimmed:sub(1, 1) == "[" then
      local ok, obj = pcall(vim.fn.json_decode, trimmed)
      if ok then
        -- Use a python one-liner for proper pretty printing since vim.fn.json_encode doesn't indent
        local pretty = vim.fn.system("python3 -c 'import json,sys; print(json.dumps(json.load(sys.stdin),indent=2))'", trimmed)
        for _, l in ipairs(vim.split(pretty, "\n", { plain = true })) do
          table.insert(lines, l)
        end
      else
        for _, l in ipairs(vim.split(text, "\n", { plain = true })) do
          table.insert(lines, l)
        end
      end
    else
      for _, l in ipairs(vim.split(text, "\n", { plain = true })) do
        table.insert(lines, l)
      end
    end
  elseif kind == "reasoning" then
    table.insert(lines, "REASONING")
    table.insert(lines, "")
    table.insert(lines, "Reasoning tokens are encrypted by the model provider.")
    table.insert(lines, "They are passed back to the API for continuation but")
    table.insert(lines, "cannot be inspected.")
  end

  vim.api.nvim_set_option_value("modifiable", true, { buf = M._preview_buf })
  vim.api.nvim_buf_set_lines(M._preview_buf, 0, -1, false, lines)
  vim.api.nvim_set_option_value("modifiable", false, { buf = M._preview_buf })

  -- Set filetype for syntax highlighting
  if kind == "tool_call" or kind == "tool_output" then
    vim.api.nvim_set_option_value("filetype", "json", { buf = M._preview_buf })
  else
    vim.api.nvim_set_option_value("filetype", "markdown", { buf = M._preview_buf })
  end

  -- Highlight search matches in the preview
  if M._search_query and M._search_query ~= "" then
    local ns = vim.api.nvim_create_namespace("prompt_browser_search")
    vim.api.nvim_buf_clear_namespace(M._preview_buf, ns, 0, -1)
    local pattern = M._search_query:lower()
    for i, line in ipairs(lines) do
      local start = 1
      while true do
        local s, e = line:lower():find(pattern, start, true)
        if not s then break end
        vim.api.nvim_buf_add_highlight(M._preview_buf, ns, "Search", i - 1, s - 1, e)
        start = e + 1
      end
    end

    -- Jump to first match in preview
    if M._preview_win and vim.api.nvim_win_is_valid(M._preview_win) then
      for i, line in ipairs(lines) do
        if line:lower():find(pattern, 1, true) then
          vim.api.nvim_win_set_cursor(M._preview_win, { i, 0 })
          break
        end
      end
    end
  end
end

-- ── Keybindings ─────────────────────────────────────────────

local function setup_keybindings()
  if not M._buf then return end
  local opts = { buffer = M._buf, nowait = true, silent = true }

  -- Enter: toggle category
  vim.keymap.set("n", "<CR>", function()
    local line = vim.api.nvim_win_get_cursor(0)[1]
    local info = M._line_map[line]
    if info and info.type == "category" then
      if M._expanded[info.name] then
        M._expanded[info.name] = nil
      else
        M._expanded[info.name] = true
      end
      M._render()
    end
  end, opts)

  -- Backspace: collapse
  vim.keymap.set("n", "<BS>", function()
    local line = vim.api.nvim_win_get_cursor(0)[1]
    local info = M._line_map[line]
    local cat_name = info and (info.category or info.name)
    if cat_name then
      M._expanded[cat_name] = nil
      M._render()
      for ln, linfo in pairs(M._line_map) do
        if linfo.type == "category" and linfo.name == cat_name then
          vim.api.nvim_win_set_cursor(0, { ln, 0 })
          break
        end
      end
    end
  end, opts)

  -- s: cycle sort
  vim.keymap.set("n", "s", function()
    M._sort_mode = M._sort_mode == "position" and "tokens" or "position"
    M._render()
  end, opts)

  -- g: cycle group mode
  vim.keymap.set("n", "g", function()
    local modes = { "flat", "role", "turn" }
    for i, mode in ipairs(modes) do
      if mode == M._group_mode then
        M._group_mode = modes[(i % #modes) + 1]
        break
      end
    end
    M._expanded = {}
    M._regroup()
    -- Expand first category
    if M._categories and #M._categories > 0 then
      M._expanded[M._categories[1].name] = true
    end
    M._render()
  end, opts)

  -- T: toggle tokens
  vim.keymap.set("n", "T", function()
    M._show_tokens = not M._show_tokens
    M._render()
  end, opts)

  -- /: search across all items in the session
  vim.keymap.set("n", "/", function()
    vim.ui.input({ prompt = "Search: " }, function(query)
      if not query or query == "" then
        M._search_matches = nil
        M._search_idx = nil
        return
      end
      -- Find all matching items
      local pattern = query:lower()
      local matches = {} -- list of line numbers in the tree that match
      local items = M._data.items or {}
      for line_nr, info in pairs(M._line_map) do
        if info.type == "entry" and info.entry then
          local entry = info.entry
          local text = ""
          if entry.kind == "message" then
            for _, t in ipairs(entry.texts or {}) do text = text .. t end
          elseif entry.text then
            text = entry.text
          end
          if text:lower():find(pattern, 1, true) then
            table.insert(matches, line_nr)
          end
        end
      end
      table.sort(matches)
      if #matches == 0 then
        vim.notify("No matches for: " .. query, vim.log.levels.INFO)
        M._search_matches = nil
        M._search_idx = nil
        return
      end
      M._search_matches = matches
      M._search_query = query
      M._search_idx = 1
      vim.api.nvim_win_set_cursor(0, { matches[1], 0 })
      vim.notify(string.format("[%d/%d] %s", 1, #matches, query), vim.log.levels.INFO)
    end)
  end, opts)

  -- n: next search match
  vim.keymap.set("n", "n", function()
    if not M._search_matches or #M._search_matches == 0 then return end
    M._search_idx = (M._search_idx % #M._search_matches) + 1
    local line = M._search_matches[M._search_idx]
    vim.api.nvim_win_set_cursor(0, { line, 0 })
    vim.notify(string.format("[%d/%d] %s", M._search_idx, #M._search_matches, M._search_query or ""), vim.log.levels.INFO)
  end, opts)

  -- N: prev search match
  vim.keymap.set("n", "N", function()
    if not M._search_matches or #M._search_matches == 0 then return end
    M._search_idx = ((M._search_idx - 2) % #M._search_matches) + 1
    local line = M._search_matches[M._search_idx]
    vim.api.nvim_win_set_cursor(0, { line, 0 })
    vim.notify(string.format("[%d/%d] %s", M._search_idx, #M._search_matches, M._search_query or ""), vim.log.levels.INFO)
  end, opts)

  -- q: close
  vim.keymap.set("n", "q", function()
    M.close()
  end, opts)

  -- ?: help
  vim.keymap.set("n", "?", function()
    local help = {
      " Prompt Browser",
      " ──────────────",
      "  <CR>  Toggle category",
      "  <BS>  Collapse category",
      "  /     Search across all items",
      "  n/N   Next/prev match",
      "  g     Cycle group (flat/role/turn)",
      "  s     Cycle sort (position/tokens)",
      "  T     Toggle token counts",
      "  q     Close",
      "  ?     This help",
      "",
      "  Press any key to close.",
    }
    local buf = vim.api.nvim_create_buf(false, true)
    vim.api.nvim_buf_set_lines(buf, 0, -1, false, help)
    vim.api.nvim_set_option_value("modifiable", false, { buf = buf })
    local win = vim.api.nvim_open_win(buf, true, {
      relative = "editor", width = 40, height = #help,
      row = math.floor((vim.o.lines - #help) / 2),
      col = math.floor((vim.o.columns - 40) / 2),
      style = "minimal", border = "rounded",
      title = " Help ", title_pos = "center",
    })
    for _, key in ipairs({ "<Esc>", "<CR>", "q", "?", "<Space>" }) do
      vim.keymap.set("n", key, function()
        if vim.api.nvim_win_is_valid(win) then vim.api.nvim_win_close(win, true) end
      end, { buffer = buf, nowait = true })
    end
  end, opts)
end

-- ── CursorMoved: auto-preview ───────────────────────────────

local function setup_cursor_autocmd()
  if not M._buf then return end
  local last_entry = nil
  vim.api.nvim_create_autocmd("CursorMoved", {
    buffer = M._buf,
    callback = function()
      local line = vim.api.nvim_win_get_cursor(0)[1]
      local info = M._line_map[line]
      if info and info.type == "entry" and info.entry ~= last_entry then
        last_entry = info.entry
        show_preview(info.entry)
      end
    end,
  })
end

-- ── Open / Close ────────────────────────────────────────────

function M.open(tree_data)
  define_highlights()

  if M._buf and vim.api.nvim_buf_is_valid(M._buf) then
    M.close()
  end

  M._data = tree_data
  M._expanded = {}
  M._line_map = {}
  M._sort_mode = "position"
  M._show_tokens = true
  M._group_mode = "flat"

  -- Compute categories from items
  M._regroup()

  -- Create tree buffer
  M._buf = vim.api.nvim_create_buf(false, true)
  vim.api.nvim_set_option_value("buftype", "nofile", { buf = M._buf })
  vim.api.nvim_set_option_value("buflisted", false, { buf = M._buf })
  vim.api.nvim_set_option_value("swapfile", false, { buf = M._buf })

  -- Create sidebar window
  vim.cmd("topleft vsplit")
  M._win = vim.api.nvim_get_current_win()
  vim.api.nvim_win_set_buf(M._win, M._buf)
  vim.api.nvim_win_set_width(M._win, 40)
  vim.api.nvim_set_option_value("winfixwidth", true, { win = M._win })
  vim.api.nvim_set_option_value("number", false, { win = M._win })
  vim.api.nvim_set_option_value("relativenumber", false, { win = M._win })
  vim.api.nvim_set_option_value("signcolumn", "no", { win = M._win })
  vim.api.nvim_set_option_value("wrap", true, { win = M._win })
  vim.api.nvim_set_option_value("cursorline", true, { win = M._win })
  vim.api.nvim_set_option_value("filetype", "prompt-browser-tree", { buf = M._buf })

  M._render()
  setup_keybindings()
  setup_cursor_autocmd()
end

function M.close()
  if M._win and vim.api.nvim_win_is_valid(M._win) then
    vim.api.nvim_win_close(M._win, true)
  end
  if M._buf and vim.api.nvim_buf_is_valid(M._buf) then
    vim.api.nvim_buf_delete(M._buf, { force = true })
  end
  if M._preview_buf and vim.api.nvim_buf_is_valid(M._preview_buf) then
    vim.api.nvim_buf_delete(M._preview_buf, { force = true })
  end
  M._win = nil
  M._buf = nil
  M._preview_buf = nil
  M._preview_win = nil
  M._data = nil
  M._line_map = {}
end

return M
