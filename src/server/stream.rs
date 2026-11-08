//! The TCP stream under each SSH connection: throttles reads with a token
//! bucket (a client pushing more than a chat needs — huge pastes, junk
//! floods — is slowed to `rate` bytes/s so it can't burn CPU on decryption;
//! normal typing and pasting never get near it), and can be killed from
//! outside (the handshake timeout).

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio::sync::Notify;
use tokio::time::{Instant, Sleep};

#[derive(Clone, Default)]
pub struct Kill(Arc<(AtomicBool, Notify)>);

impl Kill {
    pub fn kill(&self) {
        self.0.0.store(true, Ordering::Relaxed);
        self.0.1.notify_waiters();
    }
    fn killed(&self) -> bool {
        self.0.0.load(Ordering::Relaxed)
    }
}

pub struct ConnStream {
    inner: TcpStream,
    rate: f64,
    burst: f64,
    tokens: f64,
    last: Instant,
    sleep: Option<Pin<Box<Sleep>>>,
    kill: Kill,
    killed_wait: Pin<Box<dyn Future<Output = ()> + Send>>,
}

impl ConnStream {
    pub fn new(inner: TcpStream, rate: usize, burst: usize, kill: Kill) -> ConnStream {
        let k = kill.clone();
        let killed_wait = Box::pin(async move {
            loop {
                let n = k.0.1.notified();
                if k.killed() {
                    return;
                }
                n.await;
            }
        });
        ConnStream {
            inner,
            rate: rate as f64,
            burst: burst as f64,
            tokens: burst as f64,
            last: Instant::now(),
            sleep: None,
            kill,
            killed_wait,
        }
    }
}

fn gone() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::ConnectionAborted, "closed")
}

impl AsyncRead for ConnStream {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        let this = &mut *self;
        if this.kill.killed() || this.killed_wait.as_mut().poll(cx).is_ready() {
            return Poll::Ready(Ok(())); // EOF
        }
        if let Some(s) = this.sleep.as_mut() {
            if s.as_mut().poll(cx).is_pending() {
                return Poll::Pending;
            }
            this.sleep = None;
        }
        if this.rate == 0.0 {
            return Pin::new(&mut this.inner).poll_read(cx, buf);
        }
        // never read more than the burst at once
        let cap = (this.burst as usize).max(1).min(buf.remaining());
        let mut tmp = buf.take(cap);
        let r = Pin::new(&mut this.inner).poll_read(cx, &mut tmp);
        let n = tmp.filled().len();
        if let Poll::Ready(Ok(())) = r {
            unsafe { buf.assume_init(n) };
            buf.advance(n);
            let now = Instant::now();
            this.tokens = (this.tokens + now.duration_since(this.last).as_secs_f64() * this.rate).min(this.burst) - n as f64;
            this.last = now;
            if this.tokens < 0.0 {
                let d = Duration::from_secs_f64(-this.tokens / this.rate);
                this.sleep = Some(Box::pin(tokio::time::sleep(d)));
                this.tokens = 0.0;
                this.last = now + d;
            }
            return Poll::Ready(Ok(()));
        }
        r
    }
}

impl AsyncWrite for ConnStream {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, data: &[u8]) -> Poll<std::io::Result<usize>> {
        if self.kill.killed() {
            return Poll::Ready(Err(gone()));
        }
        Pin::new(&mut self.inner).poll_write(cx, data)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
