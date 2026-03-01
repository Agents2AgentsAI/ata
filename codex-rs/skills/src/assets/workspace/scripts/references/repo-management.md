# Repository Management

## Add Repository (Preferred: thick command)

Use `repo-clone` for the full safe flow (URL validation + clone policy + git clone + manifest update + audit):

```bash
python3 $WS repo-clone "https://github.com/org/repo.git" my-repo --workspace "$WID"
# Use --full for a full clone (ignores depth/filter policy)
python3 $WS repo-clone "https://github.com/org/repo.git" my-repo --workspace "$WID" --full
```

This single command handles:
- URL validation via `check-host` (HTTPS only, no embedded creds/tokens, host allowlist)
- Reading clone policy from `policies.defaultClone`
- Building git clone arguments from policy
- Cloning to `repos/<alias>/`
- LFS pull if policy allows
- Reading git state (HEAD sha, branch, default branch)
- Registering in manifest with optimistic concurrency (`--expect-version`)
- Creating notes directory
- Audit logging

Output: JSON with `repoId`, `alias`, `checkoutPath`, `state`.

## URL Validation

Before cloning manually, always validate:

```bash
python3 $WS check-host "$REPO_URL" --workspace "$WID" || exit 1
```

Validates:
- HTTPS only (rejects `http://`, `git://`, `ssh://`, `file://`)
- No embedded credentials (`user:pass@`)
- No token query params (`?token=`, `?access_token=`)
- No GitHub PAT patterns (`ghp_`, `gho_`, `github_pat_`)
- Host in `policies.repoHostsAllowlist` (absent/null = allow all, `[]` = block all)

Configure allowlist:
```bash
python3 $WS mutate '.policies.repoHostsAllowlist = ["github.com", "gitlab.com"]' --workspace "$WID"
```

## List Repositories

```bash
python3 $WS read --workspace "$WID" | jq '.repos[] | {alias, remoteUrl, state}'
```

## Update Repository

Use `python3 $WS recipe repo_update` for the full recipe, or:

```bash
ALIAS="my-repo"
REPO_PATH=$(python3 $WS resolve "@$ALIAS" --workspace "$WID")
git -C "$REPO_PATH" fetch --depth 1
DEFAULT=$(git -C "$REPO_PATH" rev-parse --abbrev-ref origin/HEAD | sed 's|origin/||')
git -C "$REPO_PATH" reset --hard "origin/$DEFAULT"
HEAD_SHA=$(git -C "$REPO_PATH" rev-parse HEAD)
NOW=$(date +%s)
python3 $WS mutate --workspace "$WID" \
  ".repos = [.repos[] | if .alias == \"$ALIAS\" then .state.headSha = \"$HEAD_SHA\" | .state.lastUpdatedAt = $NOW else . end]"
python3 $WS audit --workspace "$WID" \
  '{"op":"repo_update","targets":[{"type":"repo","alias":"'"$ALIAS"'"}]}'
```

## Pin / Unpin Repository

```bash
# Pin to specific commit
python3 $WS mutate --workspace "$WID" \
  ".repos = [.repos[] | if .alias == \"$ALIAS\" then .pin = {\"mode\":\"pinned\",\"pinnedSha\":\"$SHA\"} else . end]"

# Unpin (back to tracking)
python3 $WS mutate --workspace "$WID" \
  ".repos = [.repos[] | if .alias == \"$ALIAS\" then .pin = {\"mode\":\"tracking\"} else . end]"
```

## Remove Repository

```bash
WS_ROOT=$(python3 $WS resolve '@ws' --workspace "$WID" | sed 's|/$||')
rm -rf "$WS_ROOT/repos/$ALIAS"
python3 $WS mutate --workspace "$WID" \
  ".repos = [.repos[] | select(.alias != \"$ALIAS\")]"
python3 $WS audit --workspace "$WID" \
  '{"op":"repo_remove","targets":[{"type":"repo","alias":"'"$ALIAS"'"}]}'
```

## Shared Mirrors

Use `mirror-path` with `--reference` for faster clones:

```bash
MIRROR=$(python3 $WS mirror-path "$REPO_URL")
if [ -d "$MIRROR" ]; then
  git -C "$MIRROR" fetch --all --prune
else
  mkdir -p "$(dirname "$MIRROR")"
  git clone --mirror "$REPO_URL" "$MIRROR"
fi
git clone --reference "$MIRROR" "$REPO_URL" "$WS_ROOT/repos/$ALIAS"
```
