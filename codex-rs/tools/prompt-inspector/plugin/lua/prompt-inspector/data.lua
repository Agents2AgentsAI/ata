--- prompt-inspector/data.lua
--- Data layer: shells out to generate.py and manages caching (memory + disk).

local M = {}

--- Cached metadata (populated after first successful load_metadata call).
M._cache = nil

--- Disk cache location
local CACHE_FILE = "/tmp/prompt-inspector-cache.json"

--- Resolve the path to generate.py relative to this file.
local function resolve_generate_py()
  local this_file = debug.getinfo(1, "S").source:sub(2)
  return vim.fn.fnamemodify(this_file, ":h:h:h:h") .. "/generate.py"
end

--- Check that python3 is available.
local _python3_checked = false
local _python3_available = false

local function check_python3()
  if _python3_checked then
    return _python3_available
  end
  _python3_checked = true
  if vim.fn.executable("python3") == 1 then
    _python3_available = true
  else
    vim.notify("prompt-inspector: python3 not found in PATH", vim.log.levels.ERROR)
    _python3_available = false
  end
  return _python3_available
end

--- Directories/files whose mtimes indicate prompt content may have changed.
--- If any is newer than the cache file, the cache is stale.
local function source_sentinel_paths(codex_root)
  return {
    codex_root .. "/tools/prompt-inspector/prompt-registry.toml",
    codex_root .. "/tools/prompt-inspector/generate.py",
    codex_root .. "/core/templates",
    codex_root .. "/protocol/src/prompts",
    codex_root .. "/skills/src/assets",
    codex_root .. "/core/src/tools",
    codex_root .. "/core/prompt.md",
  }
end

--- Check if cache is stale by comparing mtimes.
--- Returns true if any source sentinel is newer than the cache file.
local function is_cache_stale(codex_root)
  local cache_stat = vim.uv.fs_stat(CACHE_FILE)
  if not cache_stat then
    return true
  end
  local cache_mtime = cache_stat.mtime.sec

  for _, path in ipairs(source_sentinel_paths(codex_root)) do
    local stat = vim.uv.fs_stat(path)
    if stat and stat.mtime.sec > cache_mtime then
      return true
    end
  end
  return false
end

--- Try to load data from disk cache.
--- Returns nil if cache doesn't exist or can't be parsed.
--- Staleness is checked separately by the caller.
local function load_disk_cache()
  if vim.fn.filereadable(CACHE_FILE) == 0 then
    return nil
  end
  local f = io.open(CACHE_FILE, "r")
  if not f then
    return nil
  end
  local raw = f:read("*a")
  f:close()
  local ok, parsed = pcall(vim.fn.json_decode, raw)
  if ok and parsed then
    return parsed
  end
  return nil
end

--- Save data to disk cache.
local function save_disk_cache(data)
  local raw = vim.fn.json_encode(data)
  local f = io.open(CACHE_FILE, "w")
  if f then
    f:write(raw)
    f:close()
  end
end

--- Run generate.py with the given extra args and call callback(result_table).
local function run_generate(codex_root, extra_args, callback)
  if not check_python3() then
    return
  end

  local generate_py = resolve_generate_py()
  if vim.fn.filereadable(generate_py) == 0 then
    vim.notify("prompt-inspector: generate.py not found at " .. generate_py, vim.log.levels.ERROR)
    return
  end

  local cmd = vim.list_extend(
    { "python3", generate_py, "--codex-root", codex_root },
    extra_args
  )

  local stdout_chunks = {}

  vim.fn.jobstart(cmd, {
    on_stdout = function(_, data, _)
      if data then
        for _, chunk in ipairs(data) do
          if chunk ~= "" then
            table.insert(stdout_chunks, chunk)
          end
        end
      end
    end,

    on_stderr = function(_, _, _) end,

    on_exit = function(_, exit_code, _)
      if exit_code ~= 0 then
        vim.schedule(function()
          vim.notify(
            string.format("prompt-inspector: generate.py exited with code %d", exit_code),
            vim.log.levels.ERROR
          )
        end)
        callback(nil)
        return
      end

      local raw = table.concat(stdout_chunks, "\n")
      local ok, parsed = pcall(vim.fn.json_decode, raw)
      if not ok or parsed == nil then
        vim.schedule(function()
          vim.notify("prompt-inspector: failed to parse JSON from generate.py", vim.log.levels.ERROR)
        end)
        callback(nil)
        return
      end

      callback(parsed)
    end,

    stdout_buffered = false,
  })
end

--- Load metadata for all prompt entries.
--- Uses memory cache > disk cache (if fresh) > generate.py.
function M.load_metadata(codex_root, callback)
  -- 1. Memory cache (instant)
  if M._cache ~= nil then
    callback(M._cache)
    return
  end

  -- 2. Disk cache — use if it exists and no source sentinel is newer
  if not is_cache_stale(codex_root) then
    local disk_data = load_disk_cache()
    if disk_data then
      M._cache = disk_data
      callback(disk_data)
      return
    end
  end

  -- 3. Generate fresh data (cache was stale or missing)
  run_generate(codex_root, { "--metadata-only" }, function(data)
    if data ~= nil then
      M._cache = data
      save_disk_cache(data)
    end
    callback(data)
  end)
end

--- Load full data for a single named entry (includes content field).
function M.load_entry(codex_root, entry_name, callback)
  run_generate(codex_root, { "--entry", entry_name }, function(data)
    callback(data)
  end)
end

--- Force regeneration of metadata, bypassing all caches.
function M.refresh(codex_root, callback)
  M._cache = nil

  run_generate(codex_root, { "--metadata-only" }, function(data)
    if data ~= nil then
      M._cache = data
      save_disk_cache(data)
    end
    callback(data)
  end)
end

return M
