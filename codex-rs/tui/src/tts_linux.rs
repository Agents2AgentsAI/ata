//! Linux-only local TTS narration via `espeak-ng`.
//!
//! Voice mode on Linux is otherwise stubbed (see `lib.rs`) because `cpal` is
//! not in the Linux dependency set — mic capture, realtime audio playback,
//! karaoke alignment, and ElevenLabs PCM streaming all depend on cpal and are
//! out of scope here.
//!
//! What this module DOES provide is the bare-minimum local narration path so
//! the reading view's `r` key, `+`/`-` speed keys, and "Speaking…" status
//! work on Linux too — primarily so CI tests can exercise the feature.
//!
//! Audio output goes through whatever `espeak-ng` plays to by default (ALSA);
//! no audio device handle ever flows through this code. CI runners without a
//! sound card will still see the child spawn and exit cleanly — `espeak-ng`
//! reports the absence as a runtime error and the parent process is
//! unaffected.

use std::collections::VecDeque;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex;

use tokio::process::Child;
use tokio::process::Command;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::mpsc::unbounded_channel;

use crate::tts_speed::TTS_SPEED_LADDER;
use crate::tts_speed::snap_to_ladder;

/// Map a `1.0`-relative speed multiplier to a words-per-minute value suitable
/// for `espeak-ng -s WPM`. 175 wpm is `espeak-ng`'s natural speaking rate;
/// the [`TTS_SPEED_LADDER`] tiers run from 0.5× (≈ 88 wpm) up to 3× (525 wpm,
/// clamped to 400 to match the macOS `say` path).
fn speed_to_wpm(speed: f64) -> u32 {
    ((175.0_f64) * speed).clamp(80.0, 400.0) as u32
}

/// Commands sent to the persistent espeak-ng worker.
enum WorkerCommand {
    /// Speak the contained text. Kills any in-flight child first so a new
    /// narrate request cleanly preempts the previous one.
    Speak(String),
    /// Signal that the speed setting in `SPEED_STATE` has changed. The worker
    /// reads the new speed on the next spawn. The currently playing child
    /// (if any) is killed so the change takes effect mid-flight.
    SetSpeed,
    /// Stop the current narration immediately. Subsequent `Speak` requests
    /// will start fresh.
    Interrupt,
}

/// Global handle to the worker task. `None` until the first narration.
static WORKER_TX: Mutex<Option<UnboundedSender<WorkerCommand>>> = Mutex::new(None);

/// Public entry: speak `text` at the current speed. Spawns the worker on
/// first call. Returns `Ok(())` once the command is enqueued; actual audio
/// happens asynchronously.
pub(crate) fn narrate(text: String) -> Result<(), String> {
    if text.trim().is_empty() {
        return Ok(());
    }
    let tx = ensure_worker()?;
    tx.send(WorkerCommand::Speak(text))
        .map_err(|err| format!("tts_linux worker is gone: {err}"))
}

/// Public entry: change the current playback speed by one tier on
/// [`TTS_SPEED_LADDER`]. Positive `delta` steps up, negative steps down.
/// Returns the new speed (the snapped tier).
pub(crate) fn step_speed(delta: f64) -> Result<f64, String> {
    let new_speed = {
        let mut state = SPEED_STATE
            .lock()
            .map_err(|err| format!("tts_linux speed mutex poisoned: {err}"))?;
        state.step(delta)
    };
    let tx = ensure_worker()?;
    let _ = tx.send(WorkerCommand::SetSpeed);
    Ok(new_speed)
}

/// Public entry: return the current playback speed, snapped to the ladder.
pub(crate) fn current_speed() -> f64 {
    SPEED_STATE.lock().map(|s| s.speed).unwrap_or(1.0)
}

/// Public entry: stop any in-flight narration.
pub(crate) fn interrupt() {
    if let Ok(guard) = WORKER_TX.lock()
        && let Some(tx) = guard.as_ref()
    {
        let _ = tx.send(WorkerCommand::Interrupt);
    }
}

/// Persistent state for the speed setting. The worker reads `speed` lazily on
/// each spawn so an in-flight `SetSpeed` and a queued `Speak` can race
/// without the spawn missing the update.
struct SpeedState {
    speed: f64,
}

impl SpeedState {
    /// Step the speed up (`delta > 0`) or down (`delta < 0`) by one tier on
    /// the ladder. Off-ladder current values are walked from the nearest tier
    /// in the chosen direction so a config-supplied `0.75` still rounds
    /// cleanly to `0.8` on `+`.
    fn step(&mut self, delta: f64) -> f64 {
        let snapped = snap_to_ladder(self.speed);
        let new_speed = if delta > 0.0 {
            // Smallest tier strictly greater than `snapped`.
            TTS_SPEED_LADDER
                .iter()
                .copied()
                .find(|&tier| tier > snapped + 1e-6)
                .unwrap_or(snapped)
        } else if delta < 0.0 {
            // Largest tier strictly less than `snapped`.
            TTS_SPEED_LADDER
                .iter()
                .copied()
                .rev()
                .find(|&tier| tier < snapped - 1e-6)
                .unwrap_or(snapped)
        } else {
            snapped
        };
        self.speed = new_speed;
        new_speed
    }
}

static SPEED_STATE: Mutex<SpeedState> = Mutex::new(SpeedState { speed: 1.0 });

/// Lazily spawn the worker task on first use, returning a handle to its
/// command channel. Subsequent calls reuse the same task.
fn ensure_worker() -> Result<UnboundedSender<WorkerCommand>, String> {
    let mut guard = WORKER_TX
        .lock()
        .map_err(|err| format!("tts_linux worker mutex poisoned: {err}"))?;
    if let Some(tx) = guard.as_ref() {
        return Ok(tx.clone());
    }
    let (tx, rx) = unbounded_channel();
    // The worker runs forever on the current tokio runtime. There is one
    // runtime per process, so a single static worker is sufficient.
    tokio::spawn(worker_loop(rx));
    *guard = Some(tx.clone());
    Ok(tx)
}

/// Long-lived task that consumes `WorkerCommand`s and shells out to
/// `espeak-ng` per `Speak`. A `Speak` waits for the previous child to exit
/// before spawning the next one so sentences play in order; `SetSpeed` and
/// `Interrupt` preempt the current child immediately.
async fn worker_loop(mut rx: tokio::sync::mpsc::UnboundedReceiver<WorkerCommand>) {
    let mut current: Option<Child> = None;
    let queue: Arc<Mutex<VecDeque<String>>> = Arc::new(Mutex::new(VecDeque::new()));

    loop {
        // If we have an in-flight child, wait for it OR for a new command.
        // `tokio::select!` picks whichever wins; a Speak/Interrupt/SetSpeed
        // landing while the child is mid-utterance cancels it.
        if let Some(mut child) = current.take() {
            tokio::select! {
                cmd = rx.recv() => {
                    // New command arrived — kill the in-flight child first.
                    kill_child(&mut child).await;
                    match cmd {
                        Some(WorkerCommand::Speak(text)) => {
                            queue.lock().map(|mut q| q.push_back(text)).ok();
                        }
                        Some(WorkerCommand::SetSpeed) => {
                            // Nothing else to do — speed was already updated in
                            // SPEED_STATE; the next spawn reads it fresh.
                        }
                        Some(WorkerCommand::Interrupt) => {
                            queue.lock().map(|mut q| q.clear()).ok();
                        }
                        None => return, // sender dropped — shouldn't happen, but exit cleanly
                    }
                }
                _ = child.wait() => {
                    // Child finished naturally; loop and pick next from queue.
                }
            }
        } else {
            match rx.recv().await {
                Some(WorkerCommand::Speak(text)) => {
                    queue.lock().map(|mut q| q.push_back(text)).ok();
                }
                Some(WorkerCommand::SetSpeed) => {
                    continue;
                }
                Some(WorkerCommand::Interrupt) => {
                    queue.lock().map(|mut q| q.clear()).ok();
                    continue;
                }
                None => return,
            }
        }

        // Spawn the next queued utterance, if any.
        let next_text = queue.lock().ok().and_then(|mut q| q.pop_front());
        if let Some(text) = next_text {
            let wpm = speed_to_wpm(current_speed());
            tracing::info!(
                "[tts_linux] spawning espeak-ng at {wpm} wpm for {} chars",
                text.len()
            );
            let spawned = Command::new("espeak-ng")
                .arg("-s")
                .arg(wpm.to_string())
                .arg("--")
                .arg(&text)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
            match spawned {
                Ok(child) => current = Some(child),
                Err(err) => {
                    tracing::warn!("[tts_linux] espeak-ng spawn failed: {err}");
                    // Drop the queue so we don't pile up unspeakable text.
                    queue.lock().map(|mut q| q.clear()).ok();
                }
            }
        }
    }
}

/// Kill an espeak-ng child cleanly. We send SIGCONT first in case the child
/// was paused (future feature); SIGTERM via `start_kill` then ends it.
async fn kill_child(child: &mut Child) {
    if let Some(pid) = child.id() {
        // SAFETY: kill(2) with SIGCONT is a well-defined POSIX syscall and
        // delivering it to a non-paused child is a no-op.
        unsafe {
            libc::kill(pid as i32, libc::SIGCONT);
        }
    }
    let _ = child.start_kill();
    let _ = child.wait().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_to_wpm_clamps() {
        assert_eq!(speed_to_wpm(0.5), 87);
        assert_eq!(speed_to_wpm(1.0), 175);
        assert_eq!(speed_to_wpm(2.0), 350);
        assert_eq!(speed_to_wpm(3.0), 400); // clamped
        assert_eq!(speed_to_wpm(0.1), 80); // clamped
    }

    #[test]
    fn speed_state_steps_up_and_down() {
        let mut state = SpeedState { speed: 1.0 };
        assert_eq!(state.step(0.1), 1.1);
        assert_eq!(state.step(0.1), 1.2);
        assert_eq!(state.step(-0.1), 1.1);
        assert_eq!(state.step(-0.1), 1.0);
        assert_eq!(state.step(-0.1), 0.9);
    }

    #[test]
    fn speed_state_clamps_to_ladder_endpoints() {
        let mut state = SpeedState { speed: 3.0 };
        // Already at top.
        assert_eq!(state.step(0.1), 3.0);
        let mut state = SpeedState { speed: 0.5 };
        // Already at bottom.
        assert_eq!(state.step(-0.1), 0.5);
    }

    #[test]
    fn speed_state_snaps_off_ladder_then_steps() {
        let mut state = SpeedState { speed: 0.75 };
        // 0.75 is not on ladder; snap to 0.8, step up to 0.9.
        assert_eq!(state.step(0.1), 0.9);
        let mut state = SpeedState { speed: 1.35 };
        // Snap to nearest tier in chosen direction, walk from there.
        assert_eq!(state.step(-0.1), 1.2);
    }
}
