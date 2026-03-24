--- prompt-browser/search.lua
--- Telescope live-grep across all prompt snapshot rollout content.

local M = {}
local data = require("prompt-browser.data")

function M.search()
  local pickers = require("telescope.pickers")
  local finders = require("telescope.finders")
  local conf = require("telescope.config").values
  local actions = require("telescope.actions")
  local action_state = require("telescope.actions.state")
  local previewers = require("telescope.previewers")

  -- Collect all rollout files that have a matching snapshot
  local sessions_dir = vim.fn.expand("~/.ata/sessions")
  local cmd = string.format(
    "find %s -name 'prompt-snapshot-*.jsonl' -type f 2>/dev/null | xargs ls -t 2>/dev/null",
    vim.fn.shellescape(sessions_dir)
  )
  local output = vim.fn.system(cmd)
  local snapshot_files = vim.split(output, "\n", { trimempty = true })

  -- Build a flat list of all items across all sessions
  local entries = {}
  for _, snap_path in ipairs(snapshot_files) do
    local rollout_path = data.find_rollout(snap_path)
    if not rollout_path then goto skip end

    local snapshots = data.parse_snapshots(snap_path)
    if #snapshots == 0 then goto skip end

    local items = data.parse_rollout(rollout_path)
    local last = snapshots[#snapshots]
    local n = last.input_items_count or #items

    local mtime = vim.fn.getftime(snap_path)
    local time_str = vim.fn.strftime("%m-%d %H:%M", mtime)
    local model = snapshots[1] and snapshots[1].model or "?"

    for i = 1, math.min(n, #items) do
      local item = items[i]
      -- Build searchable text
      local text = ""
      if item.kind == "message" then
        for _, t in ipairs(item.texts or {}) do
          text = text .. t .. "\n"
        end
      elseif item.kind == "tool_call" then
        text = item.text or ""
      elseif item.kind == "tool_output" then
        text = item.text or ""
      end

      if #text > 0 then
        local role_label = item.kind == "message" and item.role or item.kind
        local name_label = item.name or role_label
        table.insert(entries, {
          text = text,
          item = item,
          snap_path = snap_path,
          display_prefix = string.format("[%s %s] %s", time_str, model:sub(1, 8), role_label),
          name = name_label:sub(1, 50),
          ordinal = text:sub(1, 2000), -- limit for performance
        })
      end
    end
    ::skip::
  end

  pickers.new({}, {
    prompt_title = "Search Prompt Content",
    finder = finders.new_table({
      results = entries,
      entry_maker = function(entry)
        local display = string.format("%s: %s", entry.display_prefix, entry.name)
        return {
          value = entry,
          display = display,
          ordinal = entry.ordinal,
        }
      end,
    }),
    sorter = conf.generic_sorter({}),
    previewer = previewers.new_buffer_previewer({
      title = "Content",
      define_preview = function(self, entry)
        local item = entry.value.item
        local lines = {}
        if item.kind == "message" then
          table.insert(lines, item.role:upper() .. "  " .. (item.chars or 0) .. " chars")
          table.insert(lines, "")
          for _, t in ipairs(item.texts or {}) do
            for _, l in ipairs(vim.split(t, "\n", { plain = true })) do
              table.insert(lines, l)
            end
            table.insert(lines, "")
          end
        elseif item.kind == "tool_call" then
          table.insert(lines, "CALL  " .. (item.name or ""))
          table.insert(lines, "")
          local ok, obj = pcall(vim.fn.json_decode, item.text or "")
          if ok then
            local pretty = vim.fn.system("python3 -c 'import json,sys; print(json.dumps(json.load(sys.stdin),indent=2))'", item.text)
            for _, l in ipairs(vim.split(pretty, "\n", { plain = true })) do
              table.insert(lines, l)
            end
          else
            for _, l in ipairs(vim.split(item.text or "", "\n", { plain = true })) do
              table.insert(lines, l)
            end
          end
        elseif item.kind == "tool_output" then
          table.insert(lines, "OUTPUT  " .. (item.chars or 0) .. " chars")
          table.insert(lines, "")
          local text = item.text or ""
          local trimmed = text:match("^%s*(.-)%s*$")
          if trimmed:sub(1, 1) == "{" or trimmed:sub(1, 1) == "[" then
            local ok, _ = pcall(vim.fn.json_decode, trimmed)
            if ok then
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
        end
        vim.api.nvim_buf_set_lines(self.state.bufnr, 0, -1, false, lines)
      end,
    }),
    attach_mappings = function(prompt_bufnr)
      actions.select_default:replace(function()
        actions.close(prompt_bufnr)
        local selection = action_state.get_selected_entry()
        if selection and selection.value then
          -- Open the session in the browser
          local browser = require("prompt-browser")
          browser._open(selection.value.snap_path)
        end
      end)
      return true
    end,
  }):find()
end

return M
