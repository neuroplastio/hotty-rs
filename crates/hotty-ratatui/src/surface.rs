//! Surfaces as ratatui widgets.

use ratatui_core::buffer::Buffer;
use ratatui_core::layout::Rect;
use ratatui_core::widgets::{StatefulWidget, Widget};

use hotty::session::{Mode, Want};

/// The surfaces one frame wants, as its [`Surface`] widgets render: the
/// state they render into. [`Session::draw`](crate::Session::draw) makes one
/// for each frame and lays them out when the frame is drawn.
pub struct Surfaces<'a> {
    mode: Mode,
    wants: Vec<Want<'a>>,
}

impl<'a> Surfaces<'a> {
    pub(crate) fn new(mode: Mode) -> Surfaces<'a> {
        Surfaces {
            mode,
            wants: Vec::new(),
        }
    }

    /// What the terminal is, for this frame: on a host, surfaces render as
    /// themselves; anywhere else, and until it is known, as their fallback
    /// cells.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Whether surfaces render as themselves in this frame.
    pub fn native(&self) -> bool {
        self.mode == Mode::Native
    }

    /// Adds a surface by hand, as a [`Surface`] widget does.
    pub fn push(&mut self, want: Want<'a>) {
        self.wants.push(want);
    }

    pub(crate) fn into_wants(self) -> Vec<Want<'a>> {
        self.wants
    }
}

impl std::fmt::Debug for Surfaces<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Surfaces")
            .field("mode", &self.mode)
            .field("wants", &self.wants)
            .finish()
    }
}

type Fallback<'a> = Box<dyn FnOnce(Rect, &mut Buffer) + 'a>;

/// A surface as a widget: an HTML document placed on the cells it renders
/// into, on a host, and a fallback widget in those cells anywhere else
/// (SPEC §14, SDK.md §2.5).
///
/// ```no_run
/// use hotty_ratatui::Surface;
/// use ratatui::widgets::Paragraph;
/// # fn ui(frame: &mut ratatui::Frame, surfaces: &mut hotty_ratatui::Surfaces, name: &str) {
/// let area = frame.area();
/// frame.render_stateful_widget(
///     Surface::new(name, || "<p>Hello, <b>HTML</b></p>".to_string())
///         .fallback(Paragraph::new("Hello, cells")),
///     area,
///     surfaces,
/// );
/// # }
/// ```
///
/// Its document is asked for only when it must be sent: the first frame
/// that shows it, and again after the host lost it. A frame that does not
/// render it hides it, if [kept](Surface::keep), or deletes it.
#[must_use]
pub struct Surface<'a> {
    name: String,
    doc: Box<dyn FnOnce() -> String + 'a>,
    fallback: Option<Fallback<'a>>,
    whole: Option<hotty::session::Rect>,
    keep: bool,
    z: i32,
    press: bool,
    fit: bool,
    hover: bool,
    scroll: u32,
}

impl<'a> Surface<'a> {
    /// The surface named `name`, a valid surface name of the program's own
    /// ([`Session::surface`](crate::Session::surface) makes one), whose
    /// document `doc` returns.
    pub fn new(name: impl Into<String>, doc: impl FnOnce() -> String + 'a) -> Surface<'a> {
        Surface {
            name: name.into(),
            doc: Box::new(doc),
            fallback: None,
            whole: None,
            keep: false,
            z: 0,
            press: false,
            fit: false,
            hover: false,
            scroll: 0,
        }
    }

    /// What to draw in the cells where the terminal is not a host, or not
    /// known to be one yet. None leaves them blank.
    pub fn fallback(mut self, widget: impl Widget + 'a) -> Surface<'a> {
        self.fallback = Some(Box::new(move |area, buf| widget.render(area, buf)));
        self
    }

    /// The whole surface, in screen cells, where it reaches past the area it
    /// renders into: a surface scrolled partly out of view. The area is the
    /// part shown (SPEC §5.2's window).
    pub fn whole(mut self, rect: Rect) -> Surface<'a> {
        self.whole = Some(to_hotty(rect));
        self
    }

    /// When a frame does not render it, hide it rather than delete it, so it
    /// shows again without its document being sent (SPEC §5.4).
    pub fn keep(mut self, keep: bool) -> Surface<'a> {
        self.keep = keep;
        self
    }

    /// Above (greater) or below the surfaces it overlaps (SPEC §5.2).
    pub fn z(mut self, z: i32) -> Surface<'a> {
        self.z = z;
        self
    }

    /// A `press` event wherever the user presses in it (SPEC §9).
    pub fn press(mut self, on: bool) -> Surface<'a> {
        self.press = on;
        self
    }

    /// A `fit` event whenever the rows its document needs change (SPEC §5.2).
    pub fn fit(mut self, on: bool) -> Surface<'a> {
        self.fit = on;
        self
    }

    /// A `hover` event each time the element under the pointer changes
    /// (SPEC §9.4).
    pub fn hover(mut self, on: bool) -> Surface<'a> {
        self.hover = on;
        self
    }

    /// The axes the host scrolls the document along itself (SPEC §5.3): a
    /// bitmask of [`hotty::SCROLL_VERTICAL`] and [`hotty::SCROLL_HORIZONTAL`].
    pub fn scroll(mut self, axes: u32) -> Surface<'a> {
        self.scroll = axes;
        self
    }
}

impl std::fmt::Debug for Surface<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Surface")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

pub(crate) fn to_hotty(r: Rect) -> hotty::session::Rect {
    hotty::session::Rect::new(
        i32::from(r.x),
        i32::from(r.y),
        i32::from(r.width),
        i32::from(r.height),
    )
}

impl<'a> StatefulWidget for Surface<'a> {
    type State = Surfaces<'a>;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Surfaces<'a>) {
        let area = area.intersection(buf.area);
        if !state.native() {
            if let Some(f) = self.fallback {
                f(area, buf);
            }
            return;
        }
        // The host draws the surface above the cells (SPEC §5.2): they stay
        // blank, so that nothing of an earlier frame shows through.
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buf[(x, y)].reset();
            }
        }
        let clip = to_hotty(area);
        state.push(Want {
            name: self.name,
            rect: self.whole.unwrap_or(clip),
            clip: Some(clip),
            keep: self.keep,
            z: self.z,
            press: self.press,
            fit: self.fit,
            hover: self.hover,
            scroll: self.scroll,
            doc: self.doc,
        });
    }
}
