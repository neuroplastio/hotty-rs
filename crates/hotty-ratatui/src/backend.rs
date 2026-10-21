//! The terminal's output, with hotty's commands in it.

use std::io::{self, Write};

use ratatui_core::backend::{Backend as RatatuiBackend, ClearType, WindowSize};
use ratatui_core::buffer::Cell;
use ratatui_core::layout::{Position, Size};
use ratatui_crossterm::CrosstermBackend;

use crate::Session;

/// What a ratatui backend writes to, on the session's terminal: it watches
/// the frames on their way out (an erase or a scroll drops placements,
/// SPEC §5.4), and appends the session's commands to each frame, in the same
/// write, in synchronized output (SPEC §6.3), so that a placement lands with
/// the cells around it.
#[derive(Debug)]
pub struct Writer {
    session: Session,
    buf: Vec<u8>,
}

impl Writer {
    /// A writer to the session's terminal.
    pub fn new(session: &Session) -> Writer {
        Writer {
            session: session.clone(),
            buf: Vec::new(),
        }
    }
}

impl Write for Writer {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.session.watch(b);
        self.buf.extend_from_slice(b);
        Ok(b.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        let cmds = self.session.take_commands()?;
        if self.buf.is_empty() && cmds.is_empty() {
            return Ok(());
        }
        let mut out = std::mem::take(&mut self.buf);
        if !cmds.is_empty() {
            out.splice(0..0, b"\x1b[?2026h".iter().copied());
            out.extend_from_slice(cmds.as_bytes());
            out.extend_from_slice(b"\x1b[?2026l");
        }
        self.session.inner.term.write(&out)
    }
}

/// A ratatui backend on the session's terminal: crossterm's, writing
/// through a [`Writer`], but for the cursor's position, which it asks
/// through the session, since crossterm would read the terminal itself.
/// Inline viewports need that.
#[derive(Debug)]
pub struct Backend {
    inner: CrosstermBackend<Writer>,
    session: Session,
}

impl Backend {
    /// A backend on the session's terminal.
    pub fn new(session: &Session) -> Backend {
        Backend {
            inner: CrosstermBackend::new(Writer::new(session)),
            session: session.clone(),
        }
    }
}

impl RatatuiBackend for Backend {
    type Error = io::Error;

    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        self.inner.draw(content)
    }

    fn append_lines(&mut self, n: u16) -> io::Result<()> {
        self.inner.append_lines(n)
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        self.inner.hide_cursor()
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        self.inner.show_cursor()
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        RatatuiBackend::flush(&mut self.inner)?;
        let (x, y) = self.session.terminal().cursor_position()?;
        Ok(Position::new(x as u16, y as u16))
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        self.inner.set_cursor_position(position)
    }

    fn clear(&mut self) -> io::Result<()> {
        self.inner.clear()
    }

    fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
        self.inner.clear_region(clear_type)
    }

    fn size(&self) -> io::Result<Size> {
        let (w, h) = self.session.terminal().size();
        Ok(Size::new(w, h))
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        self.inner.window_size()
    }

    fn flush(&mut self) -> io::Result<()> {
        RatatuiBackend::flush(&mut self.inner)
    }

    #[cfg(feature = "scrolling-regions")]
    fn scroll_region_up(
        &mut self,
        region: std::ops::Range<u16>,
        line_count: u16,
    ) -> io::Result<()> {
        self.inner.scroll_region_up(region, line_count)
    }

    #[cfg(feature = "scrolling-regions")]
    fn scroll_region_down(
        &mut self,
        region: std::ops::Range<u16>,
        line_count: u16,
    ) -> io::Result<()> {
        self.inner.scroll_region_down(region, line_count)
    }
}
