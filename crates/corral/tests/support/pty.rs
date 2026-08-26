//! A terminal for the surfaces that only exist on one.
//!
//! The session list takes raw mode, draws a whole screen, and hands the
//! terminal to an attachment. Run on a pipe it refuses to start — correctly —
//! so a test that drove it that way would be proving the refusal rather than
//! the surface.
//!
//! Nothing here interprets what was drawn. The bytes a real terminal would
//! have rendered are kept as they arrived, and tests ask whether the text they
//! care about is among them: a second emulator in the test harness would be a
//! second thing that can be wrong.

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{Child, CommandBuilder, ExitStatus, MasterPty, PtySize, native_pty_system};

use super::SETTLE;

/// A `corral` running on its own terminal, and everything it has drawn on it.
pub struct Terminal {
    child: Box<dyn Child + Send + Sync>,
    writer: Box<dyn Write + Send>,
    drawn: Arc<Mutex<Vec<u8>>>,
    /// Held for as long as the child is. Dropping the master hangs the
    /// terminal up, and a surface whose terminal disappeared stops.
    _master: Box<dyn MasterPty + Send>,
}

impl Terminal {
    pub fn spawn(command: CommandBuilder, rows: u16, cols: u16) -> Self {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("open a pty");
        let child = pair.slave.spawn_command(command).expect("spawn corral");
        // The child holds the only other end now; keeping this one open would
        // stop the reader below ever seeing the terminal close.
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().expect("read the terminal");
        let drawn = Arc::new(Mutex::new(Vec::new()));
        let filling = Arc::clone(&drawn);
        std::thread::spawn(move || {
            let mut buffer = [0_u8; 4096];
            while let Ok(read) = reader.read(&mut buffer) {
                if read == 0 {
                    return;
                }
                filling
                    .lock()
                    .expect("the drawn bytes")
                    .extend_from_slice(&buffer[..read]);
            }
        });

        Self {
            writer: pair.master.take_writer().expect("write to the terminal"),
            child,
            drawn,
            _master: pair.master,
        }
    }

    /// Type at the surface, and report how much it had drawn when the keys
    /// went in — the anchor for asking what it drew *after* them.
    pub fn typed(&mut self, bytes: &[u8]) -> usize {
        let at = self.drawn().len();
        self.writer.write_all(bytes).expect("type at the terminal");
        self.writer.flush().expect("flush what was typed");
        at
    }

    pub fn drawn(&self) -> Vec<u8> {
        self.drawn.lock().expect("the drawn bytes").clone()
    }

    pub fn wait_for(&self, text: &str) -> usize {
        self.wait_for_after(0, text)
    }

    /// Wait until `text` has been drawn after `from`, and report where it
    /// ends.
    pub fn wait_for_after(&self, from: usize, text: &str) -> usize {
        let deadline = Instant::now() + SETTLE;
        while Instant::now() < deadline {
            let drawn = self.drawn();
            let from = from.min(drawn.len());
            if let Some(at) = find(&drawn[from..], text.as_bytes()) {
                return from + at + text.len();
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        panic!(
            "{text:?} was never drawn after byte {from}; the terminal was sent:\n{}",
            String::from_utf8_lossy(&self.drawn())
        );
    }

    pub fn between(&self, from: usize, to: usize) -> String {
        let drawn = self.drawn();
        String::from_utf8_lossy(&drawn[from.min(drawn.len())..to.min(drawn.len())]).into_owned()
    }

    /// Wait for the surface to exit, and fail rather than hang if it does not.
    ///
    /// A surface that stops answering the keyboard is one of the failures
    /// these tests exist to catch, and blocking forever on `wait` would report
    /// it as a suite that never finished.
    pub fn wait_for_exit(&mut self) -> ExitStatus {
        self.exited_within(SETTLE).unwrap_or_else(|| {
            panic!(
                "the surface never exited; the terminal was sent:\n{}",
                String::from_utf8_lossy(&self.drawn())
            )
        })
    }

    /// Whether the surface exited within `patience`.
    ///
    /// Separate from `wait_for_exit` for the tests where *how long* is the
    /// point: a surface that answers a keystroke only once some unrelated
    /// timeout expires has not answered it.
    pub fn exited_within(&mut self, patience: Duration) -> Option<ExitStatus> {
        let deadline = Instant::now() + patience;
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait().expect("the surface's state") {
                return Some(status);
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        None
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // A test that failed mid-way leaves a surface holding a terminal
        // nobody is reading; without this it stays until the harness does.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|at| at == needle)
}
