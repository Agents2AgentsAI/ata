--- prompt-inspector/telescope.lua
--- Telescope extension: fuzzy search across all prompt entries.

local M = {}

-- ---------------------------------------------------------------------------
-- Helpers
-- ---------------------------------------------------------------------------

--- Parse a source string of the form "filepath:line" into its parts.
--- @param source_str string
--- @return string, number  filepath and line number (defaults to 1)
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

--- Return a Neovim filetype string derived from a file extension.
--- @param filepath string
--- @return string
local function filetype_for(filepath)
  if not filepath then
    return ""
  end
  local ext = filepath:match("%.([^%.]+)$")
  if not ext then
    return ""
  end
  local map = {
    md   = "markdown",
    rs   = "rust",
    toml = "toml",
    lua  = "lua",
    py   = "python",
    sh   = "bash",
    ts   = "typescript",
    js   = "javascript",
    json = "json",
    yaml = "yaml",
    yml  = "yaml",
  }
  return map[ext] or ext
end

--- Format a token count the same way tree.lua does.
--- @param n number
--- @return string
local function format_tokens(n)
  n = n or 0
  if n >= 1000 then
    return string.format("%.1fk", n / 1000)
  end
  return tostring(n)
end

--- Build the display string for a single entry.
--- Format: "[Category] entry name  (Nt)"
--- @param entry table
--- @param category_name string
--- @return string
local function entry_display(entry, category_name)
  return string.format(
    "[%s] %s  (%st)",
    category_name,
    entry.name or "?",
    format_tokens(entry.tokens)
  )
end

--- Build the ordinal string for fuzzy matching.
--- Concatenates category, name, and description with spaces.
--- @param entry table
--- @param category_name string
--- @return string
local function entry_ordinal(entry, category_name)
  return table.concat({
    category_name or "",
    entry.name or "",
    entry.description or "",
  }, " ")
end

--- Flatten all entries from all categories into a single list.
--- Each item carries an extra `_category` field.
--- @param metadata table  root data object from data.lua
--- @return table[]
local function flatten_entries(metadata)
  local flat = {}
  local categories = metadata and metadata.categories or {}
  for _, cat in ipairs(categories) do
    for _, entry in ipairs(cat.entries or {}) do
      local item = vim.tbl_extend("force", {}, entry)
      item._category = cat.name or ""
      table.insert(flat, item)
    end
  end
  return flat
end

-- ---------------------------------------------------------------------------
-- M.search(codex_root)
-- ---------------------------------------------------------------------------

--- Open a Telescope fuzzy picker over all prompt entries.
--- @param codex_root string  absolute path to codex-rs/
function M.search(codex_root)
  local ok_telescope, _ = pcall(require, "telescope")
  if not ok_telescope then
    vim.notify(
      "prompt-inspector: telescope.nvim is required for fuzzy search",
      vim.log.levels.ERROR
    )
    return
  end

  local pickers   = require("telescope.pickers")
  local finders   = require("telescope.finders")
  local conf      = require("telescope.config").values
  local actions   = require("telescope.actions")
  local action_state = require("telescope.actions.state")
  local previewers = require("telescope.previewers")

  local data = require("prompt-inspector.data")

  data.load_metadata(codex_root, function(metadata)
    if metadata == nil then
      vim.notify(
        "prompt-inspector: failed to load metadata",
        vim.log.levels.ERROR
      )
      return
    end

    local entries = flatten_entries(metadata)

    -- -----------------------------------------------------------------------
    -- Previewer
    -- -----------------------------------------------------------------------

    local previewer = previewers.new_buffer_previewer({
      title = "Prompt Entry",

      define_preview = function(self, entry_item, _status)
        local buf = self.state.bufnr
        local item = entry_item.value  -- the raw entry table

        -- Build header lines
        local header = {}
        table.insert(header, string.format("Name:     %s", item.name or "?"))
        table.insert(header, string.format("Category: %s", item._category or "?"))
        table.insert(header, string.format("Tokens:   %st", format_tokens(item.tokens)))

        if item.source then
          table.insert(header, string.format("Source:   %s", item.source))
        end
        if item.git_sha then
          table.insert(header, string.format("Git SHA:  %s", item.git_sha))
        end
        if item.git_date then
          table.insert(header, string.format("Git Date: %s", item.git_date))
        end
        if item.description and item.description ~= "" then
          table.insert(header, "")
          table.insert(header, item.description)
        end
        table.insert(header, "")
        table.insert(header, string.rep("─", 60))
        table.insert(header, "")

        -- If the entry already has content (unlikely for metadata-only), use it.
        -- Otherwise fetch it asynchronously.
        local function render_lines(content_str)
          local content_lines = {}
          if content_str and content_str ~= "" then
            for _, l in ipairs(vim.split(content_str, "\n", { plain = true })) do
              table.insert(content_lines, l)
            end
          else
            table.insert(content_lines, "(no content)")
          end
          local all_lines = vim.list_extend(vim.list_extend({}, header), content_lines)
          vim.api.nvim_buf_set_lines(buf, 0, -1, false, all_lines)

          -- Set filetype for syntax highlighting
          local filepath = parse_source(item.source)
          local ft = filetype_for(filepath)
          if ft and ft ~= "" then
            vim.api.nvim_set_option_value("filetype", ft, { buf = buf })
          end
        end

        if item.content then
          render_lines(item.content)
        else
          -- Show header immediately while loading
          vim.api.nvim_buf_set_lines(buf, 0, -1, false,
            vim.list_extend(vim.list_extend({}, header), { "Loading…" })
          )
          data.load_entry(codex_root, item.name, function(full_entry)
            if not vim.api.nvim_buf_is_valid(buf) then
              return
            end
            local content = full_entry and full_entry.content or nil
            vim.schedule(function()
              render_lines(content)
            end)
          end)
        end
      end,
    })

    -- -----------------------------------------------------------------------
    -- Picker
    -- -----------------------------------------------------------------------

    pickers.new({}, {
      prompt_title = "Prompt Inspector",
      finder = finders.new_table({
        results = entries,
        entry_maker = function(item)
          return {
            value   = item,
            display = entry_display(item, item._category),
            ordinal = entry_ordinal(item, item._category),
          }
        end,
      }),
      sorter   = conf.generic_sorter({}),
      previewer = previewer,

      attach_mappings = function(prompt_bufnr, map)
        -- <CR>: open source file in current window
        actions.select_default:replace(function()
          actions.close(prompt_bufnr)
          local selection = action_state.get_selected_entry()
          if not selection then
            return
          end
          local item = selection.value
          if not item.source then
            vim.notify("prompt-inspector: no source for this entry", vim.log.levels.WARN)
            return
          end
          local filepath, line = parse_source(item.source)
          if not filepath then
            return
          end
          local full_path = codex_root .. "/" .. filepath
          if vim.fn.filereadable(full_path) == 0 then
            vim.notify("prompt-inspector: file not found: " .. full_path, vim.log.levels.ERROR)
            return
          end
          vim.cmd("edit +" .. (line or 1) .. " " .. vim.fn.fnameescape(full_path))
        end)

        -- <C-v>: open source file in vertical split
        map("i", "<C-v>", function()
          actions.close(prompt_bufnr)
          local selection = action_state.get_selected_entry()
          if not selection then
            return
          end
          local item = selection.value
          if not item.source then
            vim.notify("prompt-inspector: no source for this entry", vim.log.levels.WARN)
            return
          end
          local filepath, line = parse_source(item.source)
          if not filepath then
            return
          end
          local full_path = codex_root .. "/" .. filepath
          if vim.fn.filereadable(full_path) == 0 then
            vim.notify("prompt-inspector: file not found: " .. full_path, vim.log.levels.ERROR)
            return
          end
          vim.cmd("vsplit +" .. (line or 1) .. " " .. vim.fn.fnameescape(full_path))
        end)

        map("n", "<C-v>", function()
          actions.close(prompt_bufnr)
          local selection = action_state.get_selected_entry()
          if not selection then
            return
          end
          local item = selection.value
          if not item.source then
            vim.notify("prompt-inspector: no source for this entry", vim.log.levels.WARN)
            return
          end
          local filepath, line = parse_source(item.source)
          if not filepath then
            return
          end
          local full_path = codex_root .. "/" .. filepath
          if vim.fn.filereadable(full_path) == 0 then
            vim.notify("prompt-inspector: file not found: " .. full_path, vim.log.levels.ERROR)
            return
          end
          vim.cmd("vsplit +" .. (line or 1) .. " " .. vim.fn.fnameescape(full_path))
        end)

        -- <C-q>: send all current results to quickfix list
        map("i", "<C-q>", function()
          local picker = action_state.get_current_picker(prompt_bufnr)
          local all_results = picker:get_multi_selection()
          -- Fall back to all filtered results if nothing is multi-selected
          if vim.tbl_isempty(all_results) then
            all_results = {}
            local manager = picker.manager
            if manager then
              for entry in manager:iter() do
                table.insert(all_results, entry)
              end
            end
          end

          actions.close(prompt_bufnr)

          local qf_items = {}
          for _, sel in ipairs(all_results) do
            local item = sel.value
            if item and item.source then
              local filepath, line = parse_source(item.source)
              if filepath then
                local full_path = codex_root .. "/" .. filepath
                table.insert(qf_items, {
                  filename = full_path,
                  lnum     = line or 1,
                  col      = 1,
                  text     = string.format("[%s] %s", item._category, item.name or "?"),
                })
              end
            end
          end

          if vim.tbl_isempty(qf_items) then
            vim.notify("prompt-inspector: no entries to send to quickfix", vim.log.levels.WARN)
            return
          end

          vim.fn.setqflist(qf_items, "r")
          vim.cmd("copen")
          vim.notify(
            string.format("prompt-inspector: %d entries sent to quickfix", #qf_items),
            vim.log.levels.INFO
          )
        end)

        map("n", "<C-q>", function()
          local picker = action_state.get_current_picker(prompt_bufnr)
          local all_results = picker:get_multi_selection()
          if vim.tbl_isempty(all_results) then
            all_results = {}
            local manager = picker.manager
            if manager then
              for entry in manager:iter() do
                table.insert(all_results, entry)
              end
            end
          end

          actions.close(prompt_bufnr)

          local qf_items = {}
          for _, sel in ipairs(all_results) do
            local item = sel.value
            if item and item.source then
              local filepath, line = parse_source(item.source)
              if filepath then
                local full_path = codex_root .. "/" .. filepath
                table.insert(qf_items, {
                  filename = full_path,
                  lnum     = line or 1,
                  col      = 1,
                  text     = string.format("[%s] %s", item._category, item.name or "?"),
                })
              end
            end
          end

          if vim.tbl_isempty(qf_items) then
            vim.notify("prompt-inspector: no entries to send to quickfix", vim.log.levels.WARN)
            return
          end

          vim.fn.setqflist(qf_items, "r")
          vim.cmd("copen")
          vim.notify(
            string.format("prompt-inspector: %d entries sent to quickfix", #qf_items),
            vim.log.levels.INFO
          )
        end)

        return true
      end,
    }):find()
  end)
end

return M
