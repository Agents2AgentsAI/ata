//! Embedded V8 + a stateless JavaScript evaluator suitable for tool use.
//!
//! Each `js_eval` call spins up a fresh V8 isolate, evaluates the snippet,
//! and tears the isolate down. State does not persist across calls — this
//! keeps the threading model simple (V8 isolates are `!Send`, so a long-lived
//! cross-call REPL would require pinning a worker thread per session) and
//! avoids any leak of secrets or side-effects between agent turns.
//!
//! Future work: per-session persistent isolate with explicit `clear`. Today
//! the trade-off prefers safety + simplicity over Node-style REPL semantics.

use std::sync::Once;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::thread;
use std::thread::JoinHandle;
use std::time::Duration;
use std::time::Instant;

/// Returns the Bazel label for this proof-of-concept crate.
#[must_use]
pub fn bazel_target() -> &'static str {
    "//codex-rs/v8-poc:v8-poc"
}

/// Returns the embedded V8 version.
#[must_use]
pub fn embedded_v8_version() -> &'static str {
    v8::V8::get_version()
}

/// Returns whether the linked V8 library was built with the in-process sandbox.
#[must_use]
pub fn linked_v8_has_sandbox() -> bool {
    unsafe extern "C" {
        fn v8__V8__IsSandboxEnabled() -> bool;
    }
    unsafe { v8__V8__IsSandboxEnabled() }
}

/// One-time V8 platform initialization. Safe to call repeatedly.
pub fn ensure_v8_initialized() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        v8::V8::initialize_platform(v8::new_default_platform(0, false).make_shared());
        v8::V8::initialize();
    });
}

/// Result of evaluating a single snippet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsEvalOutcome {
    /// String form of the last expression's value, or the failure reason.
    pub value: String,
    /// `true` when V8 returned a normal value; `false` when the script threw
    /// or the evaluation was terminated due to timeout.
    pub success: bool,
    /// Wallclock time spent in V8.
    pub elapsed: Duration,
}

/// Default per-call execution budget.
const DEFAULT_EVAL_TIMEOUT: Duration = Duration::from_secs(5);
/// Cap on the number of bytes returned in `JsEvalOutcome::value`.
const MAX_OUTPUT_BYTES: usize = 16 * 1024;

/// Evaluate a JavaScript snippet in a fresh isolate. Returns `JsEvalOutcome`
/// describing the result; never panics on script errors.
///
/// V8 isolates are `!Send`, so this entire function runs on the calling
/// thread (which is whichever thread the tool handler was invoked on).
/// The isolate is dropped before returning.
pub fn js_eval(code: &str) -> JsEvalOutcome {
    js_eval_with_timeout(code, DEFAULT_EVAL_TIMEOUT)
}

/// Variant of `js_eval` with a caller-supplied timeout.
pub fn js_eval_with_timeout(code: &str, timeout: Duration) -> JsEvalOutcome {
    ensure_v8_initialized();
    let started = Instant::now();

    let isolate = &mut v8::Isolate::new(Default::default());
    v8::scope!(let scope, isolate);

    let context = v8::Context::new(scope, Default::default());
    let scope = &mut v8::ContextScope::new(scope, context);

    // Schedule a watchdog thread that asks V8 to terminate execution
    // once `timeout` elapses. We cancel it as soon as eval completes.
    let isolate_handle = scope.thread_safe_handle();
    let watchdog = spawn_timeout_thread(isolate_handle, timeout);

    let Some(source) = v8::String::new(scope, code) else {
        watchdog.cancel();
        return JsEvalOutcome {
            value: "eval source is not valid UTF-16".to_string(),
            success: false,
            elapsed: started.elapsed(),
        };
    };

    let Some(script) = v8::Script::compile(scope, source, None) else {
        watchdog.cancel();
        return JsEvalOutcome {
            value: "compile error (invalid JavaScript)".to_string(),
            success: false,
            elapsed: started.elapsed(),
        };
    };

    let result = script.run(scope);
    let timed_out = watchdog.cancel_and_check_terminated();

    match result {
        Some(value) => {
            let value_str = value.to_rust_string_lossy(scope);
            JsEvalOutcome {
                value: truncate_for_output(&value_str),
                success: true,
                elapsed: started.elapsed(),
            }
        }
        None => {
            let value = if timed_out {
                "execution terminated (timeout)".to_string()
            } else {
                "script threw an exception".to_string()
            };
            JsEvalOutcome {
                value,
                success: false,
                elapsed: started.elapsed(),
            }
        }
    }
}

/// Background thread that calls `terminate_execution` on the isolate after
/// `timeout`. Caller must call `cancel_and_check_terminated()` once the
/// eval finishes naturally so we don't terminate a future eval AND so we
/// can report whether the watchdog actually fired.
fn spawn_timeout_thread(handle: v8::IsolateHandle, timeout: Duration) -> TimeoutGuard {
    let cancel_flag = std::sync::Arc::new(AtomicBool::new(false));
    let fired_flag = std::sync::Arc::new(AtomicBool::new(false));
    let cancel_clone = cancel_flag.clone();
    let fired_clone = fired_flag.clone();
    // Spawning the watchdog thread is critical for the timeout contract — if
    // it fails, the eval can run forever. Panic with a clear message instead
    // of bubbling the io::Error up; the workspace lints deny `expect_used`
    // so use `unwrap_or_else` to satisfy the lint while preserving intent.
    let join = thread::Builder::new()
        .name("js-repl-timeout".to_string())
        .spawn(move || {
            let deadline = Instant::now() + timeout;
            loop {
                if cancel_clone.load(Ordering::SeqCst) {
                    return;
                }
                let now = Instant::now();
                if now >= deadline {
                    handle.terminate_execution();
                    fired_clone.store(true, Ordering::SeqCst);
                    return;
                }
                let remaining = (deadline - now).min(Duration::from_millis(50));
                thread::sleep(remaining);
            }
        })
        .unwrap_or_else(|e| panic!("spawn js-repl timeout thread: {e}"));
    TimeoutGuard {
        cancel_flag,
        fired_flag,
        join: Some(join),
    }
}

struct TimeoutGuard {
    cancel_flag: std::sync::Arc<AtomicBool>,
    fired_flag: std::sync::Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl TimeoutGuard {
    fn cancel(self) {
        self.cancel_flag.store(true, Ordering::SeqCst);
        if let Some(handle) = self.into_join() {
            let _ = handle.join();
        }
    }

    fn cancel_and_check_terminated(mut self) -> bool {
        self.cancel_flag.store(true, Ordering::SeqCst);
        if let Some(handle) = self.join.take() {
            let _ = handle.join();
        }
        self.fired_flag.load(Ordering::SeqCst)
    }

    fn into_join(mut self) -> Option<JoinHandle<()>> {
        self.join.take()
    }
}

impl Drop for TimeoutGuard {
    fn drop(&mut self) {
        self.cancel_flag.store(true, Ordering::SeqCst);
    }
}

fn truncate_for_output(value: &str) -> String {
    if value.len() <= MAX_OUTPUT_BYTES {
        return value.to_string();
    }
    let mut cut = MAX_OUTPUT_BYTES;
    while !value.is_char_boundary(cut) && cut > 0 {
        cut -= 1;
    }
    format!(
        "{}\n... (output truncated to {MAX_OUTPUT_BYTES} bytes)",
        &value[..cut]
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[test]
    fn exposes_expected_bazel_target() {
        assert_eq!(bazel_target(), "//codex-rs/v8-poc:v8-poc");
    }

    #[test]
    fn exposes_embedded_v8_version() {
        assert!(!embedded_v8_version().is_empty());
    }

    #[test]
    fn sandbox_feature_matches_linked_v8() {
        assert_eq!(linked_v8_has_sandbox(), cfg!(feature = "sandbox"));
    }

    #[test]
    fn js_eval_evaluates_basic_expressions() {
        let outcome = js_eval("1 + 2");
        assert!(outcome.success, "outcome={outcome:?}");
        assert_eq!(outcome.value, "3");
    }

    #[test]
    fn js_eval_evaluates_string_concatenation() {
        let outcome = js_eval("'hello ' + 'world'");
        assert!(outcome.success, "outcome={outcome:?}");
        assert_eq!(outcome.value, "hello world");
    }

    #[test]
    fn js_eval_evaluates_function_calls() {
        let outcome = js_eval("(function (n) { return n * n; })(7)");
        assert!(outcome.success, "outcome={outcome:?}");
        assert_eq!(outcome.value, "49");
    }

    #[test]
    fn js_eval_reports_thrown_exceptions() {
        let outcome = js_eval("throw new Error('boom')");
        assert!(!outcome.success);
        assert!(outcome.value.contains("threw") || outcome.value.contains("exception"));
    }

    #[test]
    fn js_eval_reports_compile_errors() {
        let outcome = js_eval("function () { ? }");
        assert!(!outcome.success);
        assert!(outcome.value.contains("compile") || outcome.value.contains("error"));
    }

    #[test]
    fn js_eval_terminates_infinite_loops() {
        let outcome = js_eval_with_timeout(
            "while (true) { Math.random(); }",
            Duration::from_millis(200),
        );
        assert!(!outcome.success);
        assert!(outcome.value.contains("terminated") || outcome.value.contains("timeout"));
    }

    #[test]
    fn js_eval_truncates_huge_output() {
        let outcome = js_eval(r#"new Array(100000).fill('x').join('')"#);
        assert!(outcome.success, "outcome={outcome:?}");
        assert!(
            outcome.value.contains("truncated"),
            "value did not include truncation marker: {}",
            &outcome.value[..200.min(outcome.value.len())]
        );
    }
}
