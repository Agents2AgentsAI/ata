--- prompt-inspector/tree.lua
--- Interactive tree buffer: categories, entries, token counts, navigation.

local M = {}

-- ---------------------------------------------------------------------------
-- State
-- ---------------------------------------------------------------------------

M._buf = nil -- tree buffer number
M._win = nil -- tree window number
M._data = nil -- loaded metadata (from data.lua)
M._expanded = {} -- set of expanded category names
M._cursor_entry = nil -- currently selected entry
M._sort_mode = "tokens" -- "name" | "tokens" | "date"
M._filter = nil -- current filter string or nil
M._on_select = nil -- callback(entry) when cursor moves to an entry
M._codex_root = nil -- codex-rs root path
M._line_map = {} -- line_nr -> { type="category"|"entry", category=..., entry=... }
M._show_tokens = true -- whether token counts are visible

-- ---------------------------------------------------------------------------
-- Highlight groups
-- ---------------------------------------------------------------------------

local function define_highlights()
  local links = {
    PromptInspectorCategory = "Title",
    PromptInspectorEntry = "Normal",
    PromptInspectorTokens = "Comment",
    PromptInspectorBroken = "ErrorMsg",
    PromptInspectorFooter = "Comment",
  }
  for group, target in pairs(links) do
    vim.api.nvim_set_hl(0, group, { link = target, default = true })
  end
end

-- ---------------------------------------------------------------------------
-- Sorting helpers
-- ---------------------------------------------------------------------------

local function sort_entries(entries, mode)
  local sorted = vim.list_extend({}, entries) -- shallow copy
  if mode == "tokens" then
    table.sort(sorted, function(a, b)
      return (a.tokens or 0) > (b.tokens or 0)
    end)
  elseif mode == "date" then
    table.sort(sorted, function(a, b)
      return (a.git_date or "") > (b.git_date or "")
    end)
  else -- "name"
    table.sort(sorted, function(a, b)
      return (a.name or "") < (b.name or "")
    end)
  end
  return sorted
end

local function sort_categories(categories, mode)
  local sorted = vim.list_extend({}, categories) -- shallow copy
  if mode == "tokens" then
    table.sort(sorted, function(a, b)
      return (a.total_tokens or 0) > (b.total_tokens or 0)
    end)
  else -- "name" or "date"
    table.sort(sorted, function(a, b)
      return (a.name or "") < (b.name or "")
    end)
  end
  return sorted
end

-- ---------------------------------------------------------------------------
-- Formatting helpers
-- ---------------------------------------------------------------------------

local function format_tokens(n)
  if n >= 1000 then
    return string.format("%.1fk", n / 1000)
  end
  return tostring(n)
end

-- ---------------------------------------------------------------------------
-- M._render()
-- ---------------------------------------------------------------------------

function M._render()
  if not M._buf or not vim.api.nvim_buf_is_valid(M._buf) then
    return
  end

  local lines = {}
  local highlights = {} -- { {line_0, hl_group, col_start, col_end}, ... }
  M._line_map = {}

  local total_tokens = M._data and M._data.total_tokens or 0
  local total_entries = 0

  local raw_categories = M._data and M._data.categories or {}
  local categories = sort_categories(raw_categories, M._sort_mode)

  for _, cat in ipairs(categories) do
    local expanded = M._expanded[cat.name] or false
    local entries = cat.entries or {}

    -- Apply filter
    local visible_entries = {}
    if M._filter and M._filter ~= "" then
      local pattern = M._filter:lower()
      for _, entry in ipairs(entries) do
        if entry.name and entry.name:lower():find(pattern, 1, true) then
          table.insert(visible_entries, entry)
        end
      end
    else
      visible_entries = entries
    end

    -- Sort
    visible_entries = sort_entries(visible_entries, M._sort_mode)

    local count = #visible_entries
    total_entries = total_entries + count

    -- Category header line
    local arrow = expanded and "\u{25bc}" or "\u{25ba}"
    local cat_tokens = cat.total_tokens or 0
    local cat_line
    if M._show_tokens then
      cat_line = string.format(
        "%s %s  %st  (%d)",
        arrow, cat.name, format_tokens(cat_tokens), count
      )
    else
      cat_line = string.format("%s %s  (%d)", arrow, cat.name, count)
    end
    table.insert(lines, cat_line)
    local line_idx = #lines
    M._line_map[line_idx] = { type = "category", category = cat.name }
    -- Highlight: category line
    table.insert(highlights, { line_idx - 1, "PromptInspectorCategory", 0, -1 })

    -- Entries (only if expanded)
    if expanded then
      for _, entry in ipairs(visible_entries) do
        local entry_line
        if entry.error then
          entry_line = string.format("  \u{2717} %s (broken)  0t", entry.name or "?")
        elseif M._show_tokens then
          entry_line = string.format(
            "  \u{25cf} %s  %st",
            entry.name or "?", format_tokens(entry.tokens or 0)
          )
        else
          entry_line = string.format("  \u{25cf} %s", entry.name or "?")
        end
        table.insert(lines, entry_line)
        local eidx = #lines
        M._line_map[eidx] = { type = "entry", category = cat.name, entry = entry }
        if entry.error then
          table.insert(highlights, { eidx - 1, "PromptInspectorBroken", 0, -1 })
        else
          -- Dim token count portion
          if M._show_tokens then
            local tok_str = format_tokens(entry.tokens or 0) .. "t"
            local col_end = #entry_line
            local col_start = col_end - #tok_str
            if col_start >= 0 then
              table.insert(highlights, { eidx - 1, "PromptInspectorTokens", col_start, col_end })
            end
          end
        end
      end
    end
  end

  -- Footer
  table.insert(lines, "")
  local footer
  if M._show_tokens then
    footer = string.format(
      "Total: ~%s tokens across %d entries",
      format_tokens(total_tokens), total_entries
    )
  else
    footer = string.format("Total: %d entries", total_entries)
  end
  table.insert(lines, footer)
  local footer_idx = #lines
  table.insert(highlights, { footer_idx - 1, "PromptInspectorFooter", 0, -1 })

  -- Sort indicator
  table.insert(lines, string.format("Sort: %s  |  s=cycle  /=filter  ?=help", M._sort_mode))
  table.insert(highlights, { #lines - 1, "PromptInspectorFooter", 0, -1 })

  -- Write to buffer
  vim.api.nvim_set_option_value("modifiable", true, { buf = M._buf })
  vim.api.nvim_buf_set_lines(M._buf, 0, -1, false, lines)
  vim.api.nvim_set_option_value("modifiable", false, { buf = M._buf })

  -- Apply highlights
  local ns = vim.api.nvim_create_namespace("prompt_inspector_tree")
  vim.api.nvim_buf_clear_namespace(M._buf, ns, 0, -1)
  for _, hl in ipairs(highlights) do
    vim.api.nvim_buf_add_highlight(M._buf, ns, hl[2], hl[1], hl[3], hl[4])
  end
end

-- ---------------------------------------------------------------------------
-- Line queries
-- ---------------------------------------------------------------------------

function M._get_entry_at_line(line)
  local info = M._line_map[line]
  if info and info.type == "entry" then
    return info.entry
  end
  return nil
end

function M._get_category_at_line(line)
  local info = M._line_map[line]
  if info and info.type == "category" then
    return info.category
  end
  return nil
end

-- ---------------------------------------------------------------------------
-- Source file parsing and opening
-- ---------------------------------------------------------------------------

local function parse_source(source_str)
  if not source_str then
    return nil, nil
  end
  local filepath, line = source_str:match("^(.+):(%d+)$")
  if filepath then
    return filepath, tonumber(line)
  end
  return source_str, 1
end

--- Open a file in the preview window (right pane), keeping the tree visible.
--- Like nvim-tree: tree stays, file opens beside it.
local function open_source_in_preview(entry)
  if not entry or not entry.source then
    vim.notify("prompt-inspector: no source for this entry", vim.log.levels.WARN)
    return
  end
  local filepath, line = parse_source(entry.source)
  if not filepath then
    return
  end
  local full_path = M._codex_root .. "/" .. filepath
  if vim.fn.filereadable(full_path) == 0 then
    vim.notify("prompt-inspector: file not found: " .. full_path, vim.log.levels.ERROR)
    return
  end
  -- Find a non-tree window to open the file in; create one if none exists
  local preview_win = nil
  for _, w in ipairs(vim.api.nvim_tabpage_list_wins(0)) do
    if w ~= M._win then
      preview_win = w
      break
    end
  end
  if preview_win then
    vim.api.nvim_set_current_win(preview_win)
  else
    -- No other window — create a split to the right
    vim.cmd("wincmd l")
    if vim.api.nvim_get_current_win() == M._win then
      vim.cmd("botright vsplit")
    end
  end
  vim.cmd("edit +" .. (line or 1) .. " " .. vim.fn.fnameescape(full_path))
end

-- ---------------------------------------------------------------------------
-- Help floating window
-- ---------------------------------------------------------------------------

local function show_help()
  local help_lines = {
    " Prompt Inspector — Keybindings",
    " ──────────────────────────────",
    "  <CR>  Toggle category / open source file",
    "  p     Preview source file (stay in sidebar)",
    "  <BS>  Collapse parent category",
    "  C-v   Open source file in vertical split",
    "  s     Cycle sort mode (name → tokens → date)",
    "  /     Search (native vim search)",
    "  T     Toggle token count display",
    "  R     Refresh data from disk",
    "  q     Close inspector",
    "  ?     Show this help",
    "",
    "  Press any key to close this help window.",
  }

  local width = 50
  local height = #help_lines
  local row = math.floor((vim.o.lines - height) / 2)
  local col = math.floor((vim.o.columns - width) / 2)

  local help_buf = vim.api.nvim_create_buf(false, true)
  vim.api.nvim_buf_set_lines(help_buf, 0, -1, false, help_lines)
  vim.api.nvim_set_option_value("modifiable", false, { buf = help_buf })

  local help_win = vim.api.nvim_open_win(help_buf, true, {
    relative = "editor",
    width = width,
    height = height,
    row = row,
    col = col,
    style = "minimal",
    border = "rounded",
    title = " Help ",
    title_pos = "center",
  })

  -- Close on any key press
  vim.keymap.set("n", "<Esc>", function()
    if vim.api.nvim_win_is_valid(help_win) then
      vim.api.nvim_win_close(help_win, true)
    end
  end, { buffer = help_buf, nowait = true })

  -- Also close on any other key
  for _, key in ipairs({ "<CR>", "q", "?", "<Space>" }) do
    vim.keymap.set("n", key, function()
      if vim.api.nvim_win_is_valid(help_win) then
        vim.api.nvim_win_close(help_win, true)
      end
    end, { buffer = help_buf, nowait = true })
  end
end

-- ---------------------------------------------------------------------------
-- Keybindings
-- ---------------------------------------------------------------------------

local function setup_keybindings()
  if not M._buf then
    return
  end
  local opts = { buffer = M._buf, nowait = true, silent = true }

  -- <CR> — toggle category or open entry source
  vim.keymap.set("n", "<CR>", function()
    local line = vim.api.nvim_win_get_cursor(0)[1]
    local cat = M._get_category_at_line(line)
    if cat then
      if M._expanded[cat] then
        M._expanded[cat] = nil
      else
        M._expanded[cat] = true
      end
      M._render()
      return
    end
    local entry = M._get_entry_at_line(line)
    if entry then
      open_source_in_preview(entry)
    end
  end, opts)

  -- p — preview: open file in adjacent window but keep cursor in sidebar
  vim.keymap.set("n", "p", function()
    local line = vim.api.nvim_win_get_cursor(0)[1]
    local entry = M._get_entry_at_line(line)
    if entry then
      open_source_in_preview(entry)
      -- Return focus to the tree
      if M._win and vim.api.nvim_win_is_valid(M._win) then
        vim.api.nvim_set_current_win(M._win)
      end
    end
  end, opts)

  -- <BS> — collapse parent category (when on an entry line)
  vim.keymap.set("n", "<BS>", function()
    local line = vim.api.nvim_win_get_cursor(0)[1]
    local info = M._line_map[line]
    if info and info.type == "entry" and info.category then
      M._expanded[info.category] = nil
      M._render()
      -- Move cursor to the category header
      for ln, linfo in pairs(M._line_map) do
        if linfo.type == "category" and linfo.category == info.category then
          vim.api.nvim_win_set_cursor(0, { ln, 0 })
          break
        end
      end
    elseif info and info.type == "category" and info.category then
      M._expanded[info.category] = nil
      M._render()
    end
  end, opts)

  -- <C-v> — open source in a new vertical split (like nvim-tree)
  vim.keymap.set("n", "<C-v>", function()
    local line = vim.api.nvim_win_get_cursor(0)[1]
    local entry = M._get_entry_at_line(line)
    if not entry or not entry.source then
      return
    end
    local filepath, ln = parse_source(entry.source)
    if not filepath then
      return
    end
    local full_path = M._codex_root .. "/" .. filepath
    if vim.fn.filereadable(full_path) == 0 then
      vim.notify("prompt-inspector: file not found: " .. full_path, vim.log.levels.ERROR)
      return
    end
    vim.cmd("vsplit +" .. (ln or 1) .. " " .. vim.fn.fnameescape(full_path))
  end, opts)

  -- s — cycle sort mode
  vim.keymap.set("n", "s", function()
    local modes = { "name", "tokens", "date" }
    for i, mode in ipairs(modes) do
      if mode == M._sort_mode then
        M._sort_mode = modes[(i % #modes) + 1]
        break
      end
    end
    M._render()
  end, opts)

  -- / — use vim's native buffer search (no custom mapping needed)

  -- R — refresh (placeholder; init.lua will wire the real callback)
  vim.keymap.set("n", "R", function()
    vim.notify("prompt-inspector: refreshing...", vim.log.levels.INFO)
    -- The refresh callback should be set by init.lua via M.set_refresh_callback
    if M._refresh_callback then
      M._refresh_callback()
    end
  end, opts)

  -- T — toggle token counts
  vim.keymap.set("n", "T", function()
    M._show_tokens = not M._show_tokens
    M._render()
  end, opts)

  -- q — close inspector
  vim.keymap.set("n", "q", function()
    M.close()
  end, opts)

  -- ? — help
  vim.keymap.set("n", "?", function()
    show_help()
  end, opts)
end

-- ---------------------------------------------------------------------------
-- CursorMoved autocmd
-- ---------------------------------------------------------------------------

local function setup_cursor_autocmd()
  if not M._buf then
    return
  end
  vim.api.nvim_create_autocmd("CursorMoved", {
    buffer = M._buf,
    callback = function()
      local line = vim.api.nvim_win_get_cursor(0)[1]
      local entry = M._get_entry_at_line(line)
      if entry and entry ~= M._cursor_entry then
        M._cursor_entry = entry
        if M._on_select then
          M._on_select(entry)
        end
      end
    end,
  })
end

-- ---------------------------------------------------------------------------
-- M.open(data, codex_root, on_select)
-- ---------------------------------------------------------------------------

function M.open(data, codex_root, on_select)
  define_highlights()

  -- Close any existing tree first
  if M._buf and vim.api.nvim_buf_is_valid(M._buf) then
    M.close()
  end

  M._data = data
  M._codex_root = codex_root
  M._on_select = on_select
  M._expanded = {}
  M._cursor_entry = nil
  M._filter = nil
  M._sort_mode = "tokens"
  M._show_tokens = true
  M._line_map = {}

  -- Create buffer
  M._buf = vim.api.nvim_create_buf(false, true)
  vim.api.nvim_set_option_value("buftype", "nofile", { buf = M._buf })
  vim.api.nvim_set_option_value("buflisted", false, { buf = M._buf })
  vim.api.nvim_set_option_value("swapfile", false, { buf = M._buf })

  -- Create window on the left
  vim.cmd("topleft vsplit")
  M._win = vim.api.nvim_get_current_win()
  vim.api.nvim_win_set_buf(M._win, M._buf)
  vim.api.nvim_win_set_width(M._win, 36)
  vim.api.nvim_set_option_value("winfixwidth", true, { win = M._win })
  vim.api.nvim_set_option_value("number", false, { win = M._win })
  vim.api.nvim_set_option_value("relativenumber", false, { win = M._win })
  vim.api.nvim_set_option_value("signcolumn", "no", { win = M._win })
  vim.api.nvim_set_option_value("wrap", true, { win = M._win })
  vim.api.nvim_set_option_value("cursorline", true, { win = M._win })

  -- Set filetype for potential ftplugin usage
  vim.api.nvim_set_option_value("filetype", "prompt-inspector-tree", { buf = M._buf })

  -- Render content
  M._render()

  -- Set up keybindings and autocmds
  setup_keybindings()
  setup_cursor_autocmd()
end

-- ---------------------------------------------------------------------------
-- M.set_refresh_callback(fn)
-- ---------------------------------------------------------------------------

function M.set_refresh_callback(fn)
  M._refresh_callback = fn
end

-- ---------------------------------------------------------------------------
-- M.update(data)
-- ---------------------------------------------------------------------------

--- Re-render with new data (used after refresh).
function M.update(data)
  M._data = data
  M._render()
end

-- ---------------------------------------------------------------------------
-- M.close()
-- ---------------------------------------------------------------------------

function M.close()
  if M._win and vim.api.nvim_win_is_valid(M._win) then
    vim.api.nvim_win_close(M._win, true)
  end
  if M._buf and vim.api.nvim_buf_is_valid(M._buf) then
    vim.api.nvim_buf_delete(M._buf, { force = true })
  end
  M._win = nil
  M._buf = nil
  M._data = nil
  M._line_map = {}
  M._cursor_entry = nil
end

return M
