#!/usr/bin/env bash
# TTS round-trip test for ATA voice mode.
#
# Verifies that ata's voice mode actually produces audible speech matching the
# agent's text output by:
#   1. Switching system audio output to BlackHole 2ch (loopback)
#   2. Recording from BlackHole input while ata speaks
#   3. Running whisper STT on the captured audio
#   4. Asserting transcript ≈ expected phrase
#
# Pre-requisites (run once):
#   brew install --cask blackhole-2ch
#   sudo killall coreaudiod   # reload HAL plugins without rebooting
#   brew install whisper-cpp switchaudio-osx
#   # Whisper model auto-downloads to $(brew --prefix)/share/whisper-cpp/ggml-base.en.bin
#
# Usage:
#   ATA_PANE=session:window.pane ./tts_roundtrip.sh "expected phrase"

set -u
EXPECTED_PHRASE="${1:-the quick brown fox jumps over the lazy dog}"
ATA_PANE="${ATA_PANE:-a2a-os:2.2}"
CAPTURE_DURATION_S="${CAPTURE_DURATION_S:-20}"
CAPTURE_WAV="/tmp/ata-tts-capture-$$.wav"
WHISPER_MODEL="$(brew --prefix)/share/whisper-cpp/ggml-base.en.bin"

cleanup() {
    rm -f "$CAPTURE_WAV"
    if [ -n "${ORIGINAL_OUTPUT:-}" ]; then
        SwitchAudioSource -s "$ORIGINAL_OUTPUT" -t output >/dev/null 2>&1 || true
        echo "Restored audio output to: $ORIGINAL_OUTPUT"
    fi
}
trap cleanup EXIT INT TERM

# Pre-flight
for tool in ffmpeg whisper-cli SwitchAudioSource; do
    if ! command -v $tool >/dev/null 2>&1; then
        echo "SKIP: $tool not installed" >&2
        exit 2
    fi
done
if ! ffmpeg -f avfoundation -list_devices true -i "" 2>&1 | grep -qi "blackhole"; then
    echo "SKIP: BlackHole 2ch input not found. Run: sudo killall coreaudiod" >&2
    exit 2
fi
if [ ! -f "$WHISPER_MODEL" ]; then
    echo "Downloading whisper base.en..." >&2
    curl -sL --create-dirs -o "$WHISPER_MODEL" \
        https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.en.bin
fi
if ! tmux capture-pane -t "$ATA_PANE" -p 2>/dev/null | grep -qE "Agents2Agents ata|Hold Space"; then
    echo "FAIL: ata not detected on pane $ATA_PANE" >&2
    exit 1
fi

# Capture current output device + switch to BlackHole
ORIGINAL_OUTPUT=$(SwitchAudioSource -c -t output)
echo "Original output: $ORIGINAL_OUTPUT"
SwitchAudioSource -s "BlackHole 2ch" -t output >/dev/null
echo "Routed output to BlackHole."

# Get the BlackHole input index
BH_IDX=$(ffmpeg -f avfoundation -list_devices true -i "" 2>&1 \
    | sed -n '/AVFoundation audio devices:/,$p' \
    | grep BlackHole | head -1 \
    | sed -E 's/.*\[([0-9]+)\].*/\1/')

# Enable voice mode in ata
tmux send-keys -t "$ATA_PANE" C-u
sleep 0.3
tmux send-keys -t "$ATA_PANE" "/voice"
sleep 0.5
tmux send-keys -t "$ATA_PANE" Enter
sleep 2

# Start recording in background
echo "Recording from BlackHole[$BH_IDX] for ${CAPTURE_DURATION_S}s..."
ffmpeg -y -loglevel error -f avfoundation -i ":${BH_IDX}" \
    -t "$CAPTURE_DURATION_S" -ac 1 -ar 16000 "$CAPTURE_WAV" &
FFPID=$!
sleep 1

# Tell ata to speak the phrase
tmux send-keys -t "$ATA_PANE" C-u
sleep 0.3
tmux send-keys -t "$ATA_PANE" "wrap exactly this in <voice> tags: $EXPECTED_PHRASE"
sleep 0.5
tmux send-keys -t "$ATA_PANE" Enter

# Wait for TTS to finish (Speaking indicator clears)
SECONDS_WAITED=0
sleep 2
while tmux capture-pane -t "$ATA_PANE" -p 2>/dev/null | grep -q "Speaking"; do
    sleep 1
    SECONDS_WAITED=$((SECONDS_WAITED + 1))
    if [ $SECONDS_WAITED -gt 25 ]; then echo "WARN: Speaking never cleared"; break; fi
done
echo "Speaking cleared after ${SECONDS_WAITED}s"

# Stop ffmpeg + turn voice off
wait $FFPID 2>/dev/null
tmux send-keys -t "$ATA_PANE" "/voice"
sleep 0.5
tmux send-keys -t "$ATA_PANE" Enter

WAV_SIZE=$(stat -f%z "$CAPTURE_WAV")
echo "Captured: $WAV_SIZE bytes"

# Transcribe
TRANSCRIPT=$(whisper-cli -m "$WHISPER_MODEL" -f "$CAPTURE_WAV" 2>/dev/null \
    | grep -E "^\[[0-9]+:[0-9]+:[0-9]+\.[0-9]+ --> " \
    | sed -E 's/^\[[^]]+\]//' \
    | tr '\n' ' ' | tr 'A-Z' 'a-z' | tr -d '.,!?' | xargs)
echo "Expected:   $EXPECTED_PHRASE"
echo "Transcript: $TRANSCRIPT"

# Assert: at least middle-of-phrase content matches
EXPECTED_LC=$(echo "$EXPECTED_PHRASE" | tr 'A-Z' 'a-z')
KEY_WORDS=$(echo "$EXPECTED_LC" | awk '{print $(NF/2), $((NF/2)+1)}')  # 2 middle words
if echo "$TRANSCRIPT" | grep -qi "$KEY_WORDS"; then
    echo "PASS: transcript contains '$KEY_WORDS'"
    exit 0
else
    echo "FAIL: transcript missing key words '$KEY_WORDS'"
    exit 1
fi
