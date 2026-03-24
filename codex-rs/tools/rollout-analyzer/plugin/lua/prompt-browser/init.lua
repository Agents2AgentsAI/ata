--- prompt-browser: browse ATA prompt snapshots in a sidebar + preview layout.
--- Matches the prompt-inspector plugin style.

local M = {}
local data = require("prompt-browser.data")
local tree = require("prompt-browser.tree")

function M.setup()
  vim.api.nvim_create_user_command("PromptBrowser", function(opts)
    M.toggle(opts.args ~= "" and opts.args or nil)
  end, { nargs = "?" })

  vim.api.nvim_create_user_command("PromptBrowserLatest", function()
    M.toggle("latest")
  end, {})

  vim.api.nvim_create_user_command("PromptBrowserPick", function()
    M.pick()
  end, {})

  vim.api.nvim_create_user_command("PromptBrowserSearch", function()
    require("prompt-browser.search").search()
  end, {})
end

--- Open a specific snapshot (or latest).
function M.toggle(snapshot_path)
  -- If already open, close
  if tree._win and vim.api.nvim_win_is_valid(tree._win) then
    tree.close()
    return
  end

  -- Resolve path
  local path = snapshot_path
  if not path or path == "latest" then
    path = data.find_latest_snapshot()
    if not path then
      vim.notify("prompt-browser: no snapshot files found in ~/.ata/sessions/", vim.log.levels.WARN)
      return
    end
  end

  M._open(path)
end

--- Telescope picker to choose a session, then open it.
function M.pick()
  -- Close existing tree if open
  if tree._win and vim.api.nvim_win_is_valid(tree._win) then
    tree.close()
  end

  require("prompt-browser.picker").pick(function(path)
    M._open(path)
  end)
end

--- Internal: open a snapshot by path.
function M._open(path)
  if vim.fn.filereadable(path) == 0 then
    vim.notify("prompt-browser: file not found: " .. path, vim.log.levels.ERROR)
    return
  end

  local rollout_path = data.find_rollout(path)
  if not rollout_path then
    vim.notify("prompt-browser: no matching rollout for " .. vim.fn.fnamemodify(path, ":t"), vim.log.levels.ERROR)
    return
  end

  local snapshots = data.parse_snapshots(path)
  local items = data.parse_rollout(rollout_path)

  if #snapshots == 0 then
    vim.notify("prompt-browser: empty snapshot file", vim.log.levels.WARN)
    return
  end

  -- Trim to last call's items
  local last_snap = snapshots[#snapshots]
  local n = last_snap.input_items_count or #items
  if n < #items then
    local trimmed = {}
    for i = 1, n do trimmed[i] = items[i] end
    items = trimmed
  end

  local total_tokens = data.total_tokens(snapshots)

  tree.open({
    items = items,
    snapshots = snapshots,
    total_tokens = total_tokens,
    snapshot_count = #snapshots,
    snapshot_path = path,
  })
end

function M.close()
  tree.close()
end

return M
