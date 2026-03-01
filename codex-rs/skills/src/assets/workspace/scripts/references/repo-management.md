# Repository Management

## Add Repository (Preferred: thick command)

Use `repo-clone` for the full safe flow (URL validation + clone policy + git clone + manifest update + audit):

```bash
ata workspace repo-clone "https://github.com/org/repo.git" my-repo --workspace "$WID"
# Use --full for a full clone (ignores depth/filter policy)
ata workspace repo-clone "https://github.com/org/repo.git" my-repo --workspace "$WID" --full
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
ata workspace check-host "$REPO_URL" --workspace "$WID" || exit 1
```

Validates:
- HTTPS only (rejects `http://`, `git://`, `ssh://`, `file://`)
- No embedded credentials (`user:pass@`)
- No token query params (`?token=`, `?access_token=`)
- No GitHub PAT patterns (`ghp_`, `gho_`, `github_pat_`)
- Host in `policies.repoHostsAllowlist` (absent/null = allow all, `[]` = block all)

Configure allowlist:
```bash
ata workspace set-field --path policies.repoHostsAllowlist --value '["github.com", "gitlab.com"]' --workspace "$WID"
```

## List Repositories

```bash
ata workspace read --workspace "$WID" | jq '.repos[] | {alias, remoteUrl, state}'
```

## Update Repository

Use `ata workspace recipe repo_update` for the full recipe, or:

```bash
ALIAS="my-repo"
REPO_PATH=$(ata workspace resolve "@$ALIAS" --workspace "$WID")
git -C "$REPO_PATH" fetch --depth 1
DEFAULT=$(git -C "$REPO_PATH" rev-parse --abbrev-ref origin/HEAD | sed 's|origin/||')
git -C "$REPO_PATH" reset --hard "origin/$DEFAULT"
HEAD_SHA=$(git -C "$REPO_PATH" rev-parse HEAD)
ata workspace repo-update-state --alias "$ALIAS" --head-sha "$HEAD_SHA" --workspace "$WID"
ata workspace audit --workspace "$WID" \
  '{"op":"repo_update","targets":[{"type":"repo","alias":"'"$ALIAS"'"}]}'
```

## Pin / Unpin Repository

```bash
# Pin to specific commit
ata workspace repo-pin --alias "$ALIAS" --sha "$SHA" --workspace "$WID"

# Unpin (back to tracking)
ata workspace repo-unpin --alias "$ALIAS" --workspace "$WID"
```

## Remove Repository

```bash
ata workspace repo-remove --alias "$ALIAS" --workspace "$WID"
```

## Shared Mirrors

Use `mirror-path` with `--reference` for faster clones:

```bash
MIRROR=$(ata workspace mirror-path "$REPO_URL")
if [ -d "$MIRROR" ]; then
  git -C "$MIRROR" fetch --all --prune
else
  mkdir -p "$(dirname "$MIRROR")"
  git clone --mirror "$REPO_URL" "$MIRROR"
fi
git clone --reference "$MIRROR" "$REPO_URL" "$WS_ROOT/repos/$ALIAS"
```
