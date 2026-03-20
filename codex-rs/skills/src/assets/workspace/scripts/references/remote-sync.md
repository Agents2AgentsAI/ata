# Remote Sync (Source Materialization)

Patterns for keeping local and remote project copies in sync. Use when working with remote GPU instances, dev servers, or any machine where the project source must match the local tree.

**Golden rule:** Never recreate source files from memory on the remote side. Always sync from the authoritative copy.

## Push Local to Remote

### rsync (Preferred)

```bash
LOCAL_PATH="/path/to/local/project/"
REMOTE="user@host:/path/to/remote/project/"

# Sync with standard exclusions
rsync -avz \
  --exclude '.git' \
  --exclude '__pycache__' \
  --exclude '*.pyc' \
  --exclude 'node_modules' \
  --exclude '.venv' \
  --exclude 'target/' \
  --exclude 'dist/' \
  --exclude '.tox' \
  --exclude '*.egg-info' \
  "$LOCAL_PATH" "$REMOTE"
```

**Respecting .gitignore patterns:**

```bash
# Use .gitignore as an exclusion filter (recommended for most projects)
rsync -avz --filter=':- .gitignore' "$LOCAL_PATH" "$REMOTE"
```

**Delete stale files on remote (mirror mode):**

```bash
# WARNING: --delete removes remote files not present locally
rsync -avz --delete --filter=':- .gitignore' "$LOCAL_PATH" "$REMOTE"
```

**Dry run first (always recommended for initial sync):**

```bash
rsync -avz --dry-run --filter=':- .gitignore' "$LOCAL_PATH" "$REMOTE"
```

### git (Alternative)

When both sides have git, push to a shared remote and pull:

```bash
# Local: push current branch
git push origin feature-branch

# Remote: pull the branch
ssh user@host "cd /path/to/remote/project && git pull origin feature-branch"
```

### scp (Single Files)

```bash
# Push a single file
scp /local/project/src/model.py user@host:/remote/project/src/model.py

# Push a directory
scp -r /local/project/src/ user@host:/remote/project/src/
```

## Pull Remote to Local

### rsync from Remote

```bash
REMOTE="user@host:/path/to/remote/project/"
LOCAL_PATH="/path/to/local/project/"

# Preview what would change before overwriting local
rsync -avz --dry-run \
  --exclude '.git' \
  --exclude '__pycache__' \
  --exclude '*.pyc' \
  --exclude 'node_modules' \
  --exclude '.venv' \
  --exclude 'target/' \
  "$REMOTE" "$LOCAL_PATH"

# Apply after review
rsync -avz \
  --exclude '.git' \
  --exclude '__pycache__' \
  --exclude '*.pyc' \
  --exclude 'node_modules' \
  --exclude '.venv' \
  --exclude 'target/' \
  "$REMOTE" "$LOCAL_PATH"
```

### Patch Mode (Safest for Code Changes)

Generate a diff on the remote, transfer, and apply locally:

```bash
# On remote: generate patch of all uncommitted changes
ssh user@host "cd /path/to/remote/project && git diff > /tmp/remote-changes.patch"

# Transfer the patch
scp user@host:/tmp/remote-changes.patch /tmp/remote-changes.patch

# Review the patch
cat /tmp/remote-changes.patch

# Apply locally
cd /path/to/local/project
git apply /tmp/remote-changes.patch
```

For untracked files on remote:

```bash
# On remote: create a tarball of new files
ssh user@host "cd /path/to/remote/project && git ls-files --others --exclude-standard | tar czf /tmp/new-files.tar.gz -T -"

# Transfer and extract locally
scp user@host:/tmp/new-files.tar.gz /tmp/new-files.tar.gz
cd /path/to/local/project
tar xzf /tmp/new-files.tar.gz
```

### scp for Specific Changed Files

```bash
# Pull specific files you know changed
scp user@host:/remote/project/src/model.py /local/project/src/model.py
scp user@host:/remote/project/configs/train.yaml /local/project/configs/train.yaml
```

## Divergence Detection

### Checksum Comparison

```bash
# Generate checksums locally
find /local/project -name '*.py' -not -path '*/__pycache__/*' -exec md5sum {} \; | sort > /tmp/local-checksums.txt

# Generate checksums on remote
ssh user@host "find /remote/project -name '*.py' -not -path '*/__pycache__/*' -exec md5sum {} \;" | sort > /tmp/remote-checksums.txt

# Compare
diff /tmp/local-checksums.txt /tmp/remote-checksums.txt
```

### rsync Dry Run

```bash
# See what differs (both directions)
echo "=== Files that would be pushed (local → remote) ==="
rsync -avz --dry-run --filter=':- .gitignore' "$LOCAL_PATH" "$REMOTE" 2>&1 | grep -v '/$'

echo "=== Files that would be pulled (remote → local) ==="
rsync -avz --dry-run --filter=':- .gitignore' "$REMOTE" "$LOCAL_PATH" 2>&1 | grep -v '/$'
```

### Directory Diff (When Both Paths Are Accessible)

```bash
# Quick: list files that differ
diff -rq /local/project/ /remote/mount/project/ \
  --exclude='.git' --exclude='__pycache__' --exclude='node_modules' --exclude='.venv'
```

### Git-Based Detection

```bash
# Compare HEAD commits (if both sides use git)
LOCAL_SHA=$(git -C /local/project rev-parse HEAD)
REMOTE_SHA=$(ssh user@host "git -C /remote/project rev-parse HEAD")

if [ "$LOCAL_SHA" != "$REMOTE_SHA" ]; then
  echo "DIVERGED: local=$LOCAL_SHA remote=$REMOTE_SHA"
  echo "Local-only commits:"
  git -C /local/project log --oneline "$REMOTE_SHA..$LOCAL_SHA"
  echo "Remote-only commits:"
  ssh user@host "git -C /remote/project log --oneline $LOCAL_SHA..$REMOTE_SHA"
else
  echo "IN SYNC at $LOCAL_SHA"
fi

# Check for uncommitted changes on remote
ssh user@host "cd /remote/project && git status --porcelain"
```

## Sync Strategies

Choose the right strategy based on your workflow:

### Mirror Mode (Local Authoritative)

Local is the single source of truth. Remote is a replica.

**When to use:** Initial deployment, CI/CD, running experiments from a known codebase.

```bash
# Full mirror: local overwrites remote completely
rsync -avz --delete --filter=':- .gitignore' "$LOCAL_PATH" "$REMOTE"
```

**Rules:**
- Never edit code on the remote directly
- Always push from local after making changes
- Use `--delete` to remove stale files on remote

### Bidirectional Mode (Active Development)

Both sides may have changes. Requires care to avoid conflicts.

**When to use:** Active development where you edit locally and also fix things on the remote.

```bash
# Step 1: detect divergence
rsync -avz --dry-run --filter=':- .gitignore' "$REMOTE" "$LOCAL_PATH" 2>&1 | tee /tmp/remote-changes.txt
rsync -avz --dry-run --filter=':- .gitignore' "$LOCAL_PATH" "$REMOTE" 2>&1 | tee /tmp/local-changes.txt

# Step 2: pull remote changes first (review the dry-run output above)
rsync -avz --filter=':- .gitignore' "$REMOTE" "$LOCAL_PATH"

# Step 3: resolve any conflicts locally, then push
rsync -avz --filter=':- .gitignore' "$LOCAL_PATH" "$REMOTE"
```

**Rules:**
- Always pull before push
- Review dry-run output before each sync
- Commit locally between sync operations for rollback safety

### Patch Mode (Safest)

Generate patches on one side, review, apply on the other. No risk of accidental overwrites.

**When to use:** When changes are small and surgical, or when you need full review before applying.

```bash
# Remote made changes — bring them to local as a patch
ssh user@host "cd /remote/project && git diff HEAD" > /tmp/remote.patch
# Review
cat /tmp/remote.patch
# Apply
git -C /local/project apply /tmp/remote.patch

# Local made changes — send to remote as a patch
git -C /local/project diff HEAD > /tmp/local.patch
scp /tmp/local.patch user@host:/tmp/local.patch
ssh user@host "cd /remote/project && git apply /tmp/local.patch"
```

**Rules:**
- Always review patches before applying
- Patches fail cleanly if there are conflicts (no partial application with `git apply`)
- Commit the applied patch immediately

## Anti-Patterns

| Anti-Pattern | Why It Is Dangerous | Do This Instead |
|--------------|---------------------|-----------------|
| Recreating files from memory on remote | Source diverges silently; no diff trail | Always `rsync` or `scp` from the authoritative copy |
| Editing both sides without a merge plan | Creates conflicts that are hard to detect | Pick one authoritative side, or use patch mode |
| Syncing without excluding build artifacts | Wastes bandwidth; may overwrite platform-specific binaries | Always use `--exclude` or `--filter=':- .gitignore'` |
| Using `rsync --delete` without `--dry-run` first | Deletes files on the target that you may still need | Always preview with `--dry-run` before `--delete` |
| Assuming remote paths exist and are writable | Sync fails silently or partially | `ssh user@host "mkdir -p /remote/path && test -w /remote/path"` |
| Syncing `.git` directory | Corrupts git state if interrupted; wastes bandwidth | Always `--exclude '.git'` |
| Forgetting to sync config/data files | Code runs on remote but with stale config | Include config dirs in sync; document what to sync |
