--- prompt-browser/picker.lua
--- Telescope picker for selecting a prompt snapshot session.

local M = {}
local data = require("prompt-browser.data")

function M.pick(on_select)
  local pickers = require("telescope.pickers")
  local finders = require("telescope.finders")
  local conf = require("telescope.config").values
  local actions = require("telescope.actions")
  local action_state = require("telescope.actions.state")

  -- Find all snapshot files
  local sessions_dir = vim.fn.expand("~/.ata/sessions")
  local cmd = string.format(
    "find %s -name 'prompt-snapshot-*.jsonl' -type f 2>/dev/null | xargs ls -t 2>/dev/null",
    vim.fn.shellescape(sessions_dir)
  )
  local output = vim.fn.system(cmd)
  local files = vim.split(output, "\n", { trimempty = true })

  if #files == 0 then
    vim.notify("prompt-browser: no snapshot files found", vim.log.levels.WARN)
    return
  end

  -- Build entries with metadata
  local entries = {}
  for _, path in ipairs(files) do
    local ok, first_line = pcall(function()
      local f = io.open(path, "r")
      if not f then return nil end
      local line = f:read("*l")
      f:close()
      return line
    end)

    local model = "?"
    local calls = 0
    local total_tok = 0

    if ok and first_line then
      local lok, obj = pcall(vim.fn.json_decode, first_line)
      if lok and obj then
        model = obj.model or "?"
      end
      -- Count lines for call count
      local lc = 0
      for _ in io.lines(path) do lc = lc + 1 end
      calls = lc

      -- Read last line for total tokens
      local last_line
      for line in io.lines(path) do last_line = line end
      if last_line then
        local lok2, last = pcall(vim.fn.json_decode, last_line)
        if lok2 and last then
          total_tok = data.total_tokens({ last })
        end
      end
    end

    local mtime = vim.fn.getftime(path)
    local time_str = vim.fn.strftime("%Y-%m-%d %H:%M", mtime)
    local display = string.format("%-16s  %-12s  %2d calls  ~%s tok  %s",
      time_str, model, calls, data.fmt_tokens(total_tok), vim.fn.fnamemodify(path, ":t"):sub(1, 30))

    table.insert(entries, {
      display = display,
      path = path,
      ordinal = time_str .. " " .. model,
      time = mtime,
    })
  end

  pickers.new({}, {
    prompt_title = "Prompt Snapshots",
    finder = finders.new_table({
      results = entries,
      entry_maker = function(entry)
        return {
          value = entry,
          display = entry.display,
          ordinal = entry.ordinal,
          path = entry.path,
        }
      end,
    }),
    sorter = conf.generic_sorter({}),
    attach_mappings = function(prompt_bufnr)
      actions.select_default:replace(function()
        actions.close(prompt_bufnr)
        local selection = action_state.get_selected_entry()
        if selection and selection.value then
          on_select(selection.value.path)
        end
      end)
      return true
    end,
  }):find()
end

return M
