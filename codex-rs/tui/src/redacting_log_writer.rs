//! Redacting writer for the file log sink.
//!
//! `RUST_LOG=trace` routes third-party HTTP/websocket spans (tungstenite,
//! reqwest, hyper) through the `fmt` file layer. Those spans can carry the
//! cleartext `Authorization` header (an API key or ChatGPT access token) and the
//! `chatgpt-account-id`. The log database layer already redacts; the file layer
//! did not. This wraps the file writer so every formatted event passes through
//! the shared [`redact_log_line`](codex_secret_patterns::redact_log_line) seam
//! before it reaches disk, regardless of which crate emitted it.

use std::io::Write;
use std::sync::Arc;
use std::sync::Mutex;

use tracing_subscriber::fmt::MakeWriter;

/// A `MakeWriter` that redacts credential material out of each formatted log
/// event before forwarding the bytes to the inner writer.
#[derive(Clone)]
pub(crate) struct RedactingMakeWriter<W> {
    inner: W,
}

impl<W> RedactingMakeWriter<W> {
    pub(crate) fn new(inner: W) -> Self {
        Self { inner }
    }
}

impl<'a, W> MakeWriter<'a> for RedactingMakeWriter<W>
where
    W: for<'b> MakeWriter<'b> + 'a,
{
    type Writer = RedactingWriter<<W as MakeWriter<'a>>::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        RedactingWriter::new(self.inner.make_writer())
    }
}

/// Buffers the bytes the `fmt` layer writes for a single event, then redacts the
/// whole buffer on drop and forwards it to the inner writer. The `fmt` layer
/// creates a fresh writer per event and drops it once the event is formatted, so
/// buffering by instance keeps redaction scoped to a complete log line.
pub(crate) struct RedactingWriter<W: Write> {
    inner: Arc<Mutex<W>>,
    buffer: Vec<u8>,
}

impl<W: Write> RedactingWriter<W> {
    fn new(inner: W) -> Self {
        Self {
            inner: Arc::new(Mutex::new(inner)),
            buffer: Vec::new(),
        }
    }

    fn flush_redacted(&mut self) -> std::io::Result<()> {
        if self.buffer.is_empty() {
            return Ok(());
        }
        let text = String::from_utf8_lossy(&self.buffer);
        let redacted = codex_secret_patterns::redact_log_line(&text);
        self.buffer.clear();
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| std::io::Error::other("redacting log writer mutex poisoned"))?;
        inner.write_all(redacted.as_bytes())
    }
}

impl<W: Write> Write for RedactingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.buffer.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.flush_redacted()?;
        self.inner
            .lock()
            .map_err(|_| std::io::Error::other("redacting log writer mutex poisoned"))?
            .flush()
    }
}

impl<W: Write> Drop for RedactingWriter<W> {
    fn drop(&mut self) {
        let _ = self.flush_redacted();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::Mutex;

    #[derive(Clone, Default)]
    struct SharedSink(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedSink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("lock").extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for SharedSink {
        type Writer = SharedSink;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    #[test]
    fn redacts_bearer_token_before_writing_to_inner() {
        let sink = SharedSink::default();
        let make = RedactingMakeWriter::new(sink.clone());
        {
            let mut w = make.make_writer();
            w.write_all(b"http request authorization: Bearer eyJhbGci.payload.signaturepart\r\nuser-agent: ata\n")
                .expect("write");
        }
        let out = String::from_utf8(sink.0.lock().expect("lock").clone()).expect("utf8");
        assert!(!out.contains("eyJhbGci"), "{out}");
        assert!(!out.contains("payload.signaturepart"), "{out}");
        assert!(out.contains("REDACTED"), "{out}");
        assert!(out.contains("user-agent: ata"), "{out}");
    }

    #[test]
    fn passes_ordinary_lines_through_unchanged() {
        let sink = SharedSink::default();
        let make = RedactingMakeWriter::new(sink.clone());
        {
            let mut w = make.make_writer();
            w.write_all(b"websocket connected\n").expect("write");
        }
        let out = String::from_utf8(sink.0.lock().expect("lock").clone()).expect("utf8");
        assert_eq!(out, "websocket connected\n");
    }
}
