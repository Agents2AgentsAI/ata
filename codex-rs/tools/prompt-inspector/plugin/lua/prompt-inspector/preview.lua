--- prompt-inspector/preview.lua
--- Right-side preview pane: renders entry metadata header and full content.

local M = {}

local data = require("prompt-inspector.data")

-- ---------------------------------------------------------------------------
-- State
-- ---------------------------------------------------------------------------

M._buf = nil        -- preview buffer number
M._win = nil        -- preview window number
M._codex_root = nil -- codex-rs root path
M._entry = nil      -- currently displayed entry

-- ---------------------------------------------------------------------------
-- Helpers
-- ---------------------------------------------------------------------------

--- Map a source file extension to a Neovim filetype.
--- @param source string  file:line formatted source string
--- @return string filetype
local function filetype_from_source(source)
  if not source then
    return "text"
  end
  -- Strip the trailing :line component first
  local filepath = source:match("^(.+):%d+$") or source
  local ext = filepath:match("%.([^%.]+)$")
  if ext == nil then
    return "text"
  end
  local ft_map = {
    md   = "markdown",
    toml = "toml",
    rs   = "rust",
    xml  = "xml",
    txt  = "text",
    lark = "text",
  }
  return ft_map[ext] or "text"
end

--- Parse a source string (path:line) into filepath and line number.
--- @param source string
--- @return string|nil, integer|nil
local function parse_source(source)
  if not source then
    return nil, nil
  end
  local filepath, line = source:match("^(.+):(%d+)$")
  if filepath then
    return filepath, tonumber(line)
  end
  return source, 1
end

-- ---------------------------------------------------------------------------
-- M.open()
-- ---------------------------------------------------------------------------

--- Create the preview buffer and window to the right of the tree.
--- @param win number|nil  Optional existing window to reuse (avoids extra split).
function M.open(win)
  -- Close any existing preview first
  if M._buf and vim.api.nvim_buf_is_valid(M._buf) then
    M._close_win_and_buf()
  end

  -- Create scratch buffer
  M._buf = vim.api.nvim_create_buf(false, true)
  vim.api.nvim_set_option_value("buftype",   "nofile", { buf = M._buf })
  vim.api.nvim_set_option_value("buflisted", false,    { buf = M._buf })
  vim.api.nvim_set_option_value("swapfile",  false,    { buf = M._buf })
  vim.api.nvim_set_option_value("modifiable", false,   { buf = M._buf })

  if win and vim.api.nvim_win_is_valid(win) then
    -- Reuse an existing window (e.g., the empty tabnew window)
    M._win = win
    vim.api.nvim_win_set_buf(M._win, M._buf)
  else
    -- Create a new split
    vim.cmd("botright vsplit")
    M._win = vim.api.nvim_get_current_win()
    vim.api.nvim_win_set_buf(M._win, M._buf)
  end

  -- Window cosmetics
  vim.api.nvim_set_option_value("number",         false, { win = M._win })
  vim.api.nvim_set_option_value("relativenumber", false, { win = M._win })
  vim.api.nvim_set_option_value("signcolumn",     "no",  { win = M._win })
  vim.api.nvim_set_option_value("wrap",           true,  { win = M._win })
  vim.api.nvim_set_option_value("cursorline",     false, { win = M._win })

  -- Buffer-local keybindings
  M._setup_keybindings()
end

-- ---------------------------------------------------------------------------
-- M.show(entry, codex_root)
-- ---------------------------------------------------------------------------

--- Display an entry in the preview pane.
--- If entry.content is absent, fetches it via data.load_entry().
--- @param entry table   entry object from the tree's on_select callback
--- @param codex_root string  absolute path to codex-rs/
function M.show(entry, codex_root)
  if not M._buf or not vim.api.nvim_buf_is_valid(M._buf) then
    return
  end

  M._codex_root = codex_root
  M._entry = entry

  if entry.content ~= nil then
    M._render(entry)
  else
    -- Metadata-only mode: fetch content asynchronously
    data.load_entry(codex_root, entry.name, function(full_entry)
      if full_entry == nil then
        vim.notify(
          "prompt-inspector: failed to load entry: " .. (entry.name or "?"),
          vim.log.levels.ERROR
        )
        return
      end
      -- Merge metadata fields that load_entry may not return
      for k, v in pairs(entry) do
        if full_entry[k] == nil then
          full_entry[k] = v
        end
      end
      -- Only render if this entry is still the active one
      if M._entry == entry then
        M._render(full_entry)
      end
    end)
  end
end

-- ---------------------------------------------------------------------------
-- M._render(entry)
-- ---------------------------------------------------------------------------

--- Write the header + content lines into the preview buffer.
--- @param entry table
function M._render(entry)
  if not M._buf or not vim.api.nvim_buf_is_valid(M._buf) then
    return
  end

  local name         = entry.name         or "(unnamed)"
  local source       = entry.source       or ""
  local tokens       = entry.tokens       or 0
  local git_modified = entry.git_modified or ""
  local git_author   = entry.git_author   or ""
  local content      = entry.content      or ""

  local header_lines = {
    "# " .. name,
    "Source: " .. source,
    string.format("Tokens: %d  |  Modified: %s  |  By: %s", tokens, git_modified, git_author),
    "",
  }

  -- Split content on newlines to get individual lines
  local content_lines = vim.split(content, "\n", { plain = true })

  local all_lines = vim.list_extend(vim.list_extend({}, header_lines), content_lines)

  vim.api.nvim_set_option_value("modifiable", true, { buf = M._buf })
  vim.api.nvim_buf_set_lines(M._buf, 0, -1, false, all_lines)
  vim.api.nvim_set_option_value("modifiable", false, { buf = M._buf })

  -- Set filetype based on source extension
  local ft = filetype_from_source(source)
  vim.api.nvim_set_option_value("filetype", ft, { buf = M._buf })
end

-- ---------------------------------------------------------------------------
-- M.close()
-- ---------------------------------------------------------------------------

--- Close preview window and buffer, clear state.
function M.close()
  M._close_win_and_buf()
  M._codex_root = nil
  M._entry = nil
end

--- Internal: close window and buffer without clearing root/entry.
function M._close_win_and_buf()
  if M._win and vim.api.nvim_win_is_valid(M._win) then
    vim.api.nvim_win_close(M._win, true)
  end
  if M._buf and vim.api.nvim_buf_is_valid(M._buf) then
    vim.api.nvim_buf_delete(M._buf, { force = true })
  end
  M._win = nil
  M._buf = nil
end

-- ---------------------------------------------------------------------------
-- Keybindings
-- ---------------------------------------------------------------------------

function M._setup_keybindings()
  if not M._buf then
    return
  end
  local opts = { buffer = M._buf, nowait = true, silent = true }

  -- q — close the inspector (preview + tree)
  vim.keymap.set("n", "q", function()
    local tree_ok, tree = pcall(require, "prompt-inspector.tree")
    if tree_ok and tree.close then
      tree.close()
    end
    M.close()
  end, opts)

  -- e — open the source file in a vsplit
  vim.keymap.set("n", "e", function()
    local entry = M._entry
    if not entry or not entry.source then
      vim.notify("prompt-inspector: no source for this entry", vim.log.levels.WARN)
      return
    end
    local filepath, line = parse_source(entry.source)
    if not filepath then
      return
    end
    local root = M._codex_root or ""
    local full_path = (root ~= "" and (root .. "/" .. filepath)) or filepath
    if vim.fn.filereadable(full_path) == 0 then
      vim.notify("prompt-inspector: file not found: " .. full_path, vim.log.levels.ERROR)
      return
    end
    vim.cmd("vsplit +" .. (line or 1) .. " " .. vim.fn.fnameescape(full_path))
  end, opts)
end

return M
