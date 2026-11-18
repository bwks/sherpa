//! Capturing the log lines code writes, for tests that check what reaches the log.
//!
//! `tracing` caches whether each callsite (an event or span in the code) is enabled, once, for
//! the whole process. While at most one dispatcher is registered, `tracing-core` takes that
//! answer from the default subscriber of whichever thread hits the callsite first. Tests run on
//! many threads, and the others have no subscriber: when one of them hits a callsite first, the
//! callsite is cached as never enabled, and a test's capturing subscriber, the only registered
//! dispatcher, never sees it. Keeping a second dispatcher registered while capturing makes
//! `tracing-core` ask every registered dispatcher instead, so a callsite that only some of them
//! want is checked on each call against its own thread's subscriber.

use std::io::{self, Write};
use std::sync::{Arc, Mutex};

use tracing::Dispatch;
use tracing::Level;
use tracing::subscriber::{self, NoSubscriber};
use tracing_subscriber::fmt::format::FmtSpan;

/// The bytes a capturing subscriber writes, shared with the test that reads them.
#[derive(Clone, Default)]
struct CapturedLog(Arc<Mutex<Vec<u8>>>);

impl Write for CapturedLog {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_| io::Error::other("log buffer lock poisoned"))?
            .extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Run `f` with every event and span it logs, at every level, written as plain text, and return
/// its result with the text. Other threads' logging is not captured.
pub(crate) fn capture_log<T>(f: impl FnOnce() -> T) -> (T, String) {
    let log = CapturedLog::default();
    let writer = log.clone();
    let capturing = tracing_subscriber::fmt()
        .with_ansi(false)
        .without_time()
        .with_max_level(Level::TRACE)
        .with_span_events(FmtSpan::FULL)
        .with_writer(move || writer.clone())
        .finish();
    // Registered before the capturing subscriber and kept until it is gone; see the module docs.
    let second = Dispatch::new(NoSubscriber::default());
    let result = subscriber::with_default(capturing, f);
    drop(second);
    let bytes = log.0.lock().map_or_default(|bytes| bytes.clone());
    (result, String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;

    /// A callsite no other test hits, so its interest is cached here.
    fn log_marker() {
        tracing::debug!("log_capture marker");
    }

    #[test]
    fn a_callsite_another_thread_hits_first_is_still_captured() {
        let ((), output) = capture_log(|| {
            // Its first hit is on a thread with no subscriber, which once cached it as never
            // enabled for every thread.
            thread::spawn(log_marker).join().unwrap();
            log_marker();
        });
        assert_eq!(output.matches("log_capture marker").count(), 1, "{output}");
    }
}
