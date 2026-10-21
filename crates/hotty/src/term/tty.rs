//! `/dev/tty`: the process's terminal, whatever its standard streams are.

use std::fmt;
use std::io;
use std::sync::Arc;

use rustix::event::{PollFd, PollFlags, poll};
use rustix::fd::{AsFd, OwnedFd};
use rustix::fs::{Mode, OFlags, open};
use rustix::termios::{OptionalActions, Termios, isatty, tcgetattr, tcgetwinsize, tcsetattr};

/// The process's terminal, opened for reading and writing.
pub struct Tty {
    fd: OwnedFd,
}

impl fmt::Debug for Tty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tty").field("fd", &self.fd).finish()
    }
}

impl Tty {
    /// Opens `/dev/tty`. Fails when the process has no terminal.
    pub fn open() -> io::Result<Tty> {
        let fd = open(
            "/dev/tty",
            OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY,
            Mode::empty(),
        )?;
        if !isatty(&fd) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "/dev/tty is not a terminal",
            ));
        }
        Ok(Tty { fd })
    }

    /// The terminal's size in cells, columns then rows, when it can be told.
    pub fn size(&self) -> Option<(u16, u16)> {
        let ws = tcgetwinsize(&self.fd).ok()?;
        (ws.ws_col > 0 && ws.ws_row > 0).then_some((ws.ws_col, ws.ws_row))
    }

    /// Puts the terminal in raw mode until the returned value is dropped.
    pub fn raw(self: &Arc<Self>) -> io::Result<RawMode> {
        let saved = tcgetattr(&self.fd)?;
        let mut raw = saved.clone();
        raw.make_raw();
        tcsetattr(&self.fd, OptionalActions::Now, &raw)?;
        Ok(RawMode {
            tty: self.clone(),
            saved,
        })
    }

    /// Writes all of `buf`.
    pub fn write(&self, buf: &[u8]) -> io::Result<usize> {
        let mut done = 0;
        while done < buf.len() {
            match rustix::io::write(&self.fd, &buf[done..]) {
                Ok(n) => done += n,
                Err(rustix::io::Errno::INTR) => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(done)
    }

    /// Reads what the terminal sends, waiting for it, or until `wake` is
    /// readable: then `ConnectionAborted`.
    pub fn read_or_wake(&self, buf: &mut [u8], wake: &impl AsFd) -> io::Result<usize> {
        loop {
            let mut fds = [
                PollFd::new(&self.fd, PollFlags::IN),
                PollFd::new(wake, PollFlags::IN),
            ];
            match poll(&mut fds, None) {
                Ok(_) => {}
                Err(rustix::io::Errno::INTR) => continue,
                Err(e) => return Err(e.into()),
            }
            if !fds[1].revents().is_empty() {
                return Err(io::ErrorKind::ConnectionAborted.into());
            }
            if fds[0].revents().is_empty() {
                continue;
            }
            return match rustix::io::read(&self.fd, &mut *buf) {
                Ok(n) => Ok(n),
                Err(rustix::io::Errno::INTR | rustix::io::Errno::AGAIN) => continue,
                Err(e) => Err(e.into()),
            };
        }
    }
}

/// The terminal in raw mode: no echo, no line editing, keys as they are
/// typed. Dropping it gives the terminal its mode back.
pub struct RawMode {
    tty: Arc<Tty>,
    saved: Termios,
}

impl fmt::Debug for RawMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RawMode")
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = tcsetattr(&self.tty.fd, OptionalActions::Now, &self.saved);
    }
}
