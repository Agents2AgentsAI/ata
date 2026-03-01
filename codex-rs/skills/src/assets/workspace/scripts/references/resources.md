# Research Resources (Papers / Datasets / Artifacts / Links)

## Add Resource

Papers, datasets, and artifacts follow the same pattern. Replace `<type>` with `papers`/`datasets`/`artifacts` and `<prefix>` with `paper`/`dataset`/`artifact`.

```bash
RESOURCE_ID=$(python3 -c "import uuid,time; print(f'<prefix>-{int(time.time())}-{uuid.uuid4().hex}')")
WS_ROOT=$(ata workspace resolve '@ws' --workspace "$WID" | sed 's|/$||')

ata workspace add-entry --collection <type> --json '{"id":"'"$RESOURCE_ID"'","title":"<title>","notesPath":"notes/<type>/'"$RESOURCE_ID"'"}' --workspace "$WID"
mkdir -p "$WS_ROOT/notes/<type>/$RESOURCE_ID"
ata workspace audit --workspace "$WID" \
  '{"op":"'"<prefix>_add"'","targets":[{"type":"'"<prefix>"'","id":"'"$RESOURCE_ID"'"}]}'
```

### Additional fields per type

- **Papers**: `authors` (array), `year` (int), `doi`, `arxiv`, `url`, `pdfArtifactId`
- **Datasets**: `name`, `url`, `license`, `artifactIds` (array)
- **Artifacts**: `kind` (required: `"pdf"`, `"model"`, `"data"`), `displayName`, `sourceUrl`, `sha256`, `sizeBytes`, `materializedPath`

## List Resources

```bash
ata workspace read --workspace "$WID" | jq '.<type>[] | {id, title}'
```

## Remove Resource

```bash
ata workspace remove-entry --collection <type> --id "$RESOURCE_ID" --workspace "$WID"
ata workspace audit --workspace "$WID" \
  '{"op":"'"<prefix>_remove"'","targets":[{"type":"'"<prefix>"'","id":"'"$RESOURCE_ID"'"}]}'
```

## Artifact Download + Checksum

```bash
WS_ROOT=$(ata workspace resolve '@ws' --workspace "$WID" | sed 's|/$||')
ART_ID=$(python3 -c "import uuid,time; print(f'artifact-{int(time.time())}-{uuid.uuid4().hex}')")
ARTIFACT_DIR="$WS_ROOT/artifacts/$ART_ID"
mkdir -p "$ARTIFACT_DIR"

curl -L -o "$ARTIFACT_DIR/blob" "$SOURCE_URL"
ACTUAL_SHA=$(shasum -a 256 "$ARTIFACT_DIR/blob" | cut -d' ' -f1)
SIZE_BYTES=$(stat -f%z "$ARTIFACT_DIR/blob" 2>/dev/null || stat -c%s "$ARTIFACT_DIR/blob")

ata workspace add-entry --collection artifacts --json '{"id":"'"$ART_ID"'","kind":"data","sourceUrl":"'"$SOURCE_URL"'","sha256":"'"$ACTUAL_SHA"'","sizeBytes":'"$SIZE_BYTES"',"materializedPath":"artifacts/'"$ART_ID"'/blob"}' --workspace "$WID"
```

## Resource Links

Links connect any two resources (repo, paper, dataset, artifact, run, index).

### Add Link

```bash
ata workspace add-entry --collection links --json '{"from":{"type":"paper","id":"'"$PAPER_ID"'"},"to":{"type":"repo","id":"'"$REPO_ID"'"},"kind":"implements"}' --workspace "$WID"
```

Common `kind` values: `implements`, `uses`, `produces`, `derived_from`, `related_to`.

### List Links

```bash
ata workspace read --workspace "$WID" | jq '.links[]'
```

### Remove Link

```bash
ata workspace remove-entry --collection links --id "$FROM_ID" --workspace "$WID"
```
