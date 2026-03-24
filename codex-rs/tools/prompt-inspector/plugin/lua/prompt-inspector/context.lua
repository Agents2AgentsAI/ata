--- prompt-inspector/context.lua
--- Runs `cargo run -p codex-cli -- debug dump-initial-context` and shows the
--- output in a scratch buffer.

local M = {}

-- ---------------------------------------------------------------------------
-- Buffer helpers
-- ---------------------------------------------------------------------------

local function create_context_buf(lines)
  local buf = vim.api.nvim_create_buf(false, true)
  vim.api.nvim_set_option_value("buftype", "nofile", { buf = buf })
  vim.api.nvim_set_option_value("buflisted", false, { buf = buf })
  vim.api.nvim_set_option_value("swapfile", false, { buf = buf })
  vim.api.nvim_set_option_value("modifiable", true, { buf = buf })
  -- Don't set a buffer name to avoid E95 on re-open

  vim.api.nvim_buf_set_lines(buf, 0, -1, false, lines)

  vim.api.nvim_set_option_value("modifiable", false, { buf = buf })
  vim.api.nvim_set_option_value("filetype", "markdown", { buf = buf })

  -- q to close
  vim.keymap.set("n", "q", function()
    if vim.api.nvim_buf_is_valid(buf) then
      vim.api.nvim_buf_delete(buf, { force = true })
    end
  end, { buffer = buf, nowait = true, silent = true })

  return buf
end

local function open_buf_in_window(buf)
  vim.cmd("split")
  local win = vim.api.nvim_get_current_win()
  vim.api.nvim_win_set_buf(win, buf)
end

-- ---------------------------------------------------------------------------
-- M.show(codex_root)
-- ---------------------------------------------------------------------------

--- Show the initial context dump in a scratch buffer.
---@param codex_root string  Absolute path to the codex-rs workspace root.
function M.show(codex_root)
  vim.notify("Loading context...", vim.log.levels.INFO)

  -- Check that cargo is available before launching the job.
  if vim.fn.executable("cargo") == 0 then
    local buf = create_context_buf({ "Error: cargo not found in PATH" })
    open_buf_in_window(buf)
    return
  end

  local stdout_lines = {}

  vim.fn.jobstart(
    { "cargo", "run", "-p", "codex-cli", "--", "debug", "dump-initial-context" },
    {
      cwd = codex_root,

      on_stdout = function(_, data, _)
        if data then
          for _, line in ipairs(data) do
            table.insert(stdout_lines, line)
          end
        end
      end,

      on_exit = function(_, exit_code, _)
        local lines
        if exit_code ~= 0 then
          lines = {
            "Error: cargo command failed — the dump-initial-context command may not be implemented yet.",
            "See Task 11 in the prompt inspector plan.",
          }
        else
          -- Strip trailing empty string that jobstart appends
          if stdout_lines[#stdout_lines] == "" then
            table.remove(stdout_lines)
          end
          lines = stdout_lines
        end

        vim.schedule(function()
          local buf = create_context_buf(lines)
          open_buf_in_window(buf)
        end)
      end,
    }
  )
end

return M
