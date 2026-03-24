local M = {}

M._config = {}

function M.setup(opts)
  opts = opts or {}

  assert(type(opts.codex_root) == "string", "prompt-inspector: opts.codex_root is required and must be a string")

  local codex_root = vim.fn.expand(opts.codex_root)
  M._config = { codex_root = codex_root }

  vim.api.nvim_create_user_command("PromptInspector", function()
    M.toggle()
  end, {})

  vim.api.nvim_create_user_command("PromptSearch", function()
    require("prompt-inspector.telescope").search(codex_root)
  end, {})

  vim.api.nvim_create_user_command("PromptContext", function()
    require("prompt-inspector.context").show(codex_root)
  end, {})

  vim.api.nvim_create_user_command("PromptRefresh", function()
    local tree = require("prompt-inspector.tree")
    require("prompt-inspector.data").refresh(codex_root, function(new_data)
      tree.update(new_data)
    end)
  end, {})

  vim.keymap.set("n", "<leader>ip", "<cmd>PromptInspector<cr>", { desc = "Prompt Inspector" })
  vim.keymap.set("n", "<leader>is", "<cmd>PromptSearch<cr>", { desc = "Search Prompts" })
  vim.keymap.set("n", "<leader>if", "<cmd>PromptContext<cr>", { desc = "Full Agent Context" })

  local ok, wk = pcall(require, "which-key")
  if ok then
    wk.add({ { "<leader>i", group = "inspect" } })
  end
end

--- Internal: open sidebar with given data.
function M._open_with_data(tree_mod, data_mod, codex_root, data)
  local function on_select(entry)
    -- No-op: files open via <CR>/p in the adjacent window.
  end

  tree_mod.open(data, codex_root, on_select)

  tree_mod.set_refresh_callback(function()
    data_mod.refresh(codex_root, function(new_data)
      tree_mod.update(new_data)
    end)
  end)

  if tree_mod._win and vim.api.nvim_win_is_valid(tree_mod._win) then
    vim.api.nvim_set_current_win(tree_mod._win)
  end
end

--- Toggle the inspector sidebar. Behaves like nvim-tree:
--- - Opens a sidebar on the left, files open in the adjacent window
--- - Calling again closes the sidebar
function M.toggle()
  local tree_mod = require("prompt-inspector.tree")

  -- If tree is already open, close it
  if tree_mod._win and vim.api.nvim_win_is_valid(tree_mod._win) then
    tree_mod.close()
    return
  end

  local codex_root = M._config.codex_root
  local data_mod = require("prompt-inspector.data")

  if data_mod._cache then
    -- Cache hit — open immediately
    M._open_with_data(tree_mod, data_mod, codex_root, data_mod._cache)
  else
    -- Cache miss — open sidebar immediately with "Loading..." then populate
    local loading_data = {
      total_tokens = 0,
      categories = { { name = "Loading...", total_tokens = 0, entries = {} } },
    }
    M._open_with_data(tree_mod, data_mod, codex_root, loading_data)

    data_mod.load_metadata(codex_root, function(data)
      if data then
        tree_mod.update(data)
      end
    end)
  end
end

function M.close()
  require("prompt-inspector.tree").close()
end

return M
