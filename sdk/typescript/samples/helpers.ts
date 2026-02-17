import path from "node:path";

export function ataPathOverride() {
  return (
    process.env.CODEX_EXECUTABLE ??
    path.join(process.cwd(), "..", "..", "codex-rs", "target", "debug", "ata")
  );
}
