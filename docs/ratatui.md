---
group: Reference
parent: hotty-rs
order: 20
nav: ratatui
path: hotty-rs/ratatui
description: surfaces as ratatui widgets, and crossterm's events beside hotty's
---

# hotty in a ratatui program

`hotty-ratatui` puts hotty's surfaces in a [ratatui](https://ratatui.rs)
program. A `Surface` is a widget: on a host the host draws its HTML above
the cells it renders into; anywhere else it draws its fallback, any ratatui
widget. So one program has both renditions, and the user's terminal picks.

## Install

```toml
[dependencies]
hotty-ratatui = { git = "https://github.com/neuroplastio/hotty-rs" }
ratatui = "0.30"
```

It is for ratatui 0.30 on crossterm, on Unix. A program that enables
ratatui's `scrolling-regions` feature enables hotty-ratatui's too, since it
adds methods to the backend.

## A program

```rust
use hotty_ratatui::{Event, Surface};
use ratatui::crossterm::event::{Event as Input, KeyCode};
use ratatui::widgets::Paragraph;

const CARD: &str = "<p id=n>0</p><button id=ok>Click</button>";

fn main() -> std::io::Result<()> {
    let (mut terminal, session) = hotty_ratatui::init("hello")?;
    let card = session.surface("card");
    let mut clicks = 0;
    loop {
        session.draw(&mut terminal, |frame, surfaces| {
            let cells = format!("{clicks} clicks · q quits");
            frame.render_stateful_widget(
                Surface::new(&card, || CARD.into())
                    .fallback(Paragraph::new(cells)),
                frame.area(),
                surfaces,
            );
        })?;
        match session.read()? {
            Event::Input(Input::Key(k)) => match k.code {
                KeyCode::Char('q') => break,
                // A field keeps the keyboard until the program takes it.
                KeyCode::Esc => session.send(hotty::blur(&card)),
                _ => {}
            },
            Event::Hotty(e) if e.kind == "click" => {
                clicks += 1;
                // The surface changes in place, with the next frame.
                session.send(hotty::set_text(&card, "n", clicks));
            }
            _ => {} // Ready, Relayout, a resize: draw again
        }
    }
    hotty_ratatui::restore(&mut terminal, &session)
}
```

- **`init`** opens the terminal as `ratatui::init` does (raw mode, the
  alternate screen, a panic hook that restores it), starts a session, and
  waits until it knows whether the terminal is a host, so the first frame
  draws the right rendition. `restore` deletes the surfaces and gives the
  terminal back.
- **`session.draw`** is `terminal.draw` with the frame's surfaces: the
  `Surface` widgets it renders are the surfaces on screen. A new one gets
  its document, a moved one is placed again, one the frame no longer
  renders is deleted, or hidden if it is `keep(true)`. Their commands go
  out with the frame's cells.
- **The document is asked for only when it must be sent**: the first
  frame that shows the surface, and after the host lost it. Change it
  afterwards with deltas: `session.send(hotty::delta(…))`.

## Events

`session.read()` and `session.poll(timeout)` stand where crossterm's
`event::read()` and `event::poll()` stood:

| `Event` | what it is |
| --- | --- |
| `Input(ev)` | crossterm's event, as crossterm would read it: keys, the mouse, pastes, focus, resizes. Hand it to the handler the program has |
| `Ready(mode)` | the session knows the terminal: `Mode::Native` (a host) or `Mode::Text`. Draw |
| `Hotty(ev)` | something the user did in a surface: `click`, `submit`, `change`, `input`, … with its target and detail |
| `Relayout { lost }` | the screen was erased or scrolled under the surfaces, or the host lost a document: draw, which places them again. With `lost`, the host may have lost the resources (`hotty::res`) its documents refer to as well: `session.send` them again before drawing, so that they reach it ahead of the documents |
| `Error(reply)`, `Ack(reply)` | a command the host refused, or the ok of one the program numbered |
| `Pong(took)` | the answer to `session.ping()`: how long the terminal took to take a frame |

**The session reads the terminal itself.** A host answers on the
terminal's input, among the keys, and crossterm's reader would take its
answers for keys. So the session cuts hotty's sequences out of the input
and parses the rest into crossterm's events. While a session is open,
nothing else may read the terminal: not crossterm's `event::read`, and
not `cursor::position`. The terminal `init` makes asks for the cursor's
position through the session, which ratatui's `Terminal::clear` and inline
viewports need.

## The keyboard

A surface with a field takes the keyboard when the user clicks into it:
the keys go to the field, and the ones it doesn't use (Escape, a key its
`data-keys` gives the program) come back as `Input`. The program takes the
keyboard back with a blur, as the program above does on Escape:
`session.send(hotty::blur(&card))`.

A field edits the same in cells as on a host when the program gives both
the same keymap: `hotty::TERMINAL_KEYS` in the surface's `data-keys`, and a
`hotty::Field` with `hotty::resolve(…, TERMINAL_KEYS)` in the cells.

## The widget

| `Surface` | what it does |
| --- | --- |
| `new(name, doc)` | the surface `name` (`session.surface("card")` makes one of this program's own), its document from `doc` |
| `.fallback(widget)` | what to draw in the cells where the terminal is not a host, or not known to be one yet |
| `.whole(rect)` | the whole surface, where it reaches past the area it renders into: a surface scrolled partly out of view |
| `.keep(true)` | hide it, rather than delete it, when a frame doesn't render it |
| `.press(true)`, `.fit(true)`, `.hover(true)` | ask for `press`, `fit` and `hover` events |
| `.scroll(axes)` | let the host scroll the document itself: `hotty::SCROLL_VERTICAL`, `SCROLL_HORIZONTAL` |
| `.z(n)` | above or below the surfaces it overlaps |

## Example

[`board`](https://github.com/neuroplastio/hotty-rs/blob/main/crates/hotty-ratatui/examples/board.rs)
is a full-screen program with a surface among its cells: buttons and a
form on a host, a paragraph in cells, and a log of what it heard.

```sh
cargo run -p hotty-ratatui --example board
```
