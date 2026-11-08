//! Front-ends for SSH sessions: a lightweight full-screen terminal UI, a
//! plain line mode for old or minimal terminals, and a JSON Lines bot mode.

pub mod agent;
pub mod format;
pub mod keys;
pub mod plain;
pub mod style;
pub mod term;
pub mod text;

use crate::chat::Client;
use std::pin::Pin;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

/// What the SSH layer delivers to a running UI.
#[derive(Debug)]
pub enum Input {
    Data(Vec<u8>),
    Resize(usize, usize),
    Eof,
}

/// A session's terminal: input from the client, output to it.
pub struct Io {
    pub input: mpsc::Receiver<Input>,
    pub out: Pin<Box<dyn AsyncWrite + Send>>,
}

impl Io {
    /// Writes everything; false once the connection is gone.
    pub async fn write(&mut self, data: &[u8]) -> bool {
        if data.is_empty() {
            return true;
        }
        self.out.write_all(data).await.is_ok() && self.out.flush().await.is_ok()
    }
}

/// The reason the hub dropped the client, if it did.
pub fn done_reason(c: &Client) -> Option<String> {
    c.done.borrow().clone()
}

#[cfg(test)]
pub mod testio {
    use super::*;
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll};

    /// Captures everything a UI writes.
    #[derive(Clone, Default)]
    pub struct Capture(pub Arc<Mutex<Vec<u8>>>);

    impl AsyncWrite for Capture {
        fn poll_write(self: Pin<&mut Self>, _: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Poll::Ready(Ok(buf.len()))
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    impl Capture {
        pub fn text(&self) -> String {
            String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
        }
    }

    pub fn io() -> (Io, mpsc::Sender<Input>, Capture) {
        let (tx, rx) = mpsc::channel(64);
        let cap = Capture::default();
        (Io { input: rx, out: Box::pin(cap.clone()) }, tx, cap)
    }
}
