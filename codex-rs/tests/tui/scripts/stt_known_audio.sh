#!/usr/bin/env bash
# STT round-trip test using `say` for a reference WAV.
#
# This doesn't test ata's TTS or STT pipelines end-to-end (no audio loopback),
# but it proves the toolchain we'd use for a real round-trip test works:
#   - macOS `say` generates audio from text
#   - Apple Speech.framework (via osascript) OR whisper STTs the audio
#   - Transcript matches reference text
#
# Exit codes:
#   0  PASS
#   1  FAIL
#   2  SKIP (no STT available)

set -u
REF_PHRASE="${1:-the quick brown fox jumps over the lazy dog}"
REF_AIFF="/tmp/stt-ref-$$.aiff"
REF_WAV="/tmp/stt-ref-$$.wav"

# 1. Generate reference audio with `say`.
echo "Generating reference: '$REF_PHRASE'"
say -o "$REF_AIFF" "$REF_PHRASE"
if [ ! -s "$REF_AIFF" ]; then
    echo "FAIL: say did not produce audio" >&2
    exit 1
fi
echo "Reference AIFF: $(stat -f%z "$REF_AIFF") bytes"

# Convert to 16kHz mono WAV (whisper expects this).
afconvert -f WAVE -d LEI16@16000 -c 1 "$REF_AIFF" "$REF_WAV" 2>/dev/null

# 2. Run STT.
if command -v whisper-cli >/dev/null 2>&1; then
    MODEL="$(brew --prefix)/share/whisper-cpp/ggml-base.en.bin"
    if [ ! -f "$MODEL" ]; then
        echo "Downloading whisper base.en model..." >&2
        curl -sL --create-dirs -o "$MODEL" \
            https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin
    fi
    # whisper-cli prints lines as `[hh:mm:ss.mmm --> hh:mm:ss.mmm]  text`
    TRANSCRIPT=$(whisper-cli -m "$MODEL" -f "$REF_WAV" 2>/dev/null \
        | grep -E "^\[[0-9]+:[0-9]+:[0-9]+\.[0-9]+ --> " \
        | sed -E 's/^\[[^]]+\]//' \
        | tr '\n' ' ' \
        | tr 'A-Z' 'a-z' | tr -d '.,!?' | xargs)
else
    echo "SKIP: no whisper installed. brew install whisper-cpp" >&2
    rm -f "$REF_AIFF" "$REF_WAV"
    exit 2
fi

echo "Transcript: $TRANSCRIPT"
echo "Reference:  $REF_PHRASE"

# 3. Assert: at least key words match.
REF_LC=$(echo "$REF_PHRASE" | tr 'A-Z' 'a-z')
KEY_WORD=$(echo "$REF_LC" | awk '{print $4}')  # pick a middle word
if echo "$TRANSCRIPT" | grep -q -i "$KEY_WORD"; then
    echo "PASS: STT transcript contains key word '$KEY_WORD'"
    rm -f "$REF_AIFF" "$REF_WAV" "${REF_WAV%.wav}.txt"
    exit 0
else
    echo "FAIL: STT transcript missing key word '$KEY_WORD'" >&2
    exit 1
fi
