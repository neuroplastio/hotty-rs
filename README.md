# hotty-rs

hotty-rs is the Rust SDK for [hotty](https://github.com/neuroplastio/hotty),
HTML over the TTY. With it, a terminal program shows small HTML documents,
called surfaces, on rectangles of cells, and hears what the user does in
them. It follows the hotty repository's SDK.md: the same two layers, names
and defaults as the other SDKs, and every SDK section of the conformance
vectors.

It speaks hotty 0.4: a host's replies and events carry msgpack bodies (SPEC
§3.3), its query lists the version it speaks (`a=q:n=1:v=0.4`, SPEC §4), and
keys are named as myelin key names 0.1 names them (SPEC §10.4). A terminal of
another hotty version is no host to it, and the program draws in cells.

| crate | what it is | SDK.md |
| --- | --- | --- |
| `hotty` | the wire layer: builders, `encode`, `Decoder`, `Scanner`, `Detector`, replies, events and capabilities, keys and keymaps; a `Field` edited as a host edits one; `Link`, `Session` and `Terminal` | §3, §4.1, §4.3, §4.6 |
| `hotty-ratatui` | hotty for a [ratatui](https://ratatui.rs) program: a `Surface` widget, a session that reads the terminal and hands back crossterm's events, a backend | §4.3 |

0.4.x speaks hotty 0.4; the patch number is this repository's own.

## A program that prints

```rust
use hotty::{Placement, Terminal};

fn main() -> std::io::Result<()> {
    let Ok(t) = Terminal::open("hello") else {
        println!("Hello."); // no terminal: a pipe, a cron job
        return Ok(());
    };
    if t.detect() {
        t.line_start(); // surfaces are placed at the cursor
        t.print("hello", "<p>Hello, <b>HTML</b> in the terminal.</p>", &Placement::new(40, 1))?;
        t.fence().ok(); // the host's replies, before the shell reads the terminal
    } else {
        println!("Hello."); // a terminal that is not a host, or not of this version
    }
    Ok(())
}
```

`Terminal` opens `/dev/tty`, whatever the standard streams are, reads it on a
thread of its own, and puts it in raw mode while a call waits for an answer.
It is Unix only, behind the default feature `term`.

## A ratatui program

```rust
use hotty_ratatui::{Event, Surface};
use ratatui::crossterm::event::{Event as Input, KeyCode};
use ratatui::widgets::Paragraph;

let (mut terminal, session) = hotty_ratatui::init("hello")?;
let card = session.surface("card");
loop {
    session.draw(&mut terminal, |frame, surfaces| {
        frame.render_stateful_widget(
            Surface::new(&card, || "<p>Hello, <b>HTML</b>. <button id=ok>OK</button></p>".into())
                .fallback(Paragraph::new("Hello, cells. Press q.")),
            frame.area(),
            surfaces,
        );
    })?;
    match session.read()? {
        Event::Input(Input::Key(k)) if k.code == KeyCode::Char('q') => break,
        Event::Hotty(e) if e.kind == "click" => break,
        _ => {} // Ready, Relayout, a resize: draw again
    }
}
hotty_ratatui::restore(&mut terminal, &session)?;
```

A `Surface` renders its document on a host, where the cells under it stay
blank and the host draws above them, and its fallback widget anywhere else.
`session.draw` lays the frame's surfaces out: a new one gets its document and
a placement, a moved one a placement, one the frame no longer renders is
hidden or deleted. Their commands go out with the frame's cells. Deltas
(`session.send(hotty::set_text(…))`) go with the next frame.

**The session reads the terminal.** The host answers on the terminal's
input, among the keys, and crossterm's reader would take its answers for
keys. So the session reads the terminal itself, cuts hotty's sequences out,
and hands the rest back as crossterm's events, `Event::Input`: a program
reads `session.read()` and `session.poll()` where it read crossterm's
`event::read()` and `event::poll()`, and keeps its handlers. It must not
read the terminal any other way while the session is open, and that
includes crossterm's `cursor::position()`. `hotty_ratatui::Backend` asks for
the cursor's position through the session, which ratatui's `Terminal::clear`
and inline viewports need; `init` makes a terminal on it.

A program that enables ratatui's `scrolling-regions` feature enables
hotty-ratatui's too, since it adds methods to the backend trait.

## Text fields in cells

A program that draws its fields in cells edits them with the keymap it gives
its surfaces' fields, so a field edits the same on a host and off one:

```rust
use hotty::{Field, INSERT, TERMINAL_KEYS, decode_keys, key_text, platform_of, resolve};

let keys = resolve(&platform_of(&caps), false, [TERMINAL_KEYS]);
let mut f = Field::default();
for key in decode_keys(&read).into_iter().flatten() {
    match keys.lookup(&key) {
        Some(INSERT) => { f.type_text(&key_text(&key).unwrap()); }
        Some("submit") => submit(&f.value),
        Some(a) if keys.selects(&key) => { f.extend(a); }
        Some(a) => { f.do_action(a); }
        None => on_key(&key), // not the field's: the program's own keymap has it
    }
}
```

## Examples

| example | what it shows |
| --- | --- |
| [`hello`](crates/hotty/examples/hello.rs) | open, detect, `line_start`, `print`, `fence`: the smallest program |
| [`progress`](crates/hotty/examples/progress.rs) | a bar moved by one delta of `var` and `text` ops; three renditions: surface, cells, pipe |
| [`board`](crates/hotty-ratatui/examples/board.rs) | a full-screen ratatui program: a surface among its cells, clicks and a form, deltas, keys for the cells |

```sh
cargo run -p hotty --example hello
cargo run -p hotty-ratatui --example board
```

## Names

The canonical names of SDK.md, in Rust's case (its Appendix A). Where a name
is a keyword or clashes:
- the capabilities' and a Platform's `mod` is `modifier`;
- a Field's `Do` and `Type` are `do_action` and `type_text`;
- the builders return a `Cmd` (or a `Doc`, a `Query`), since `Command` is a
  `command` event's detail; options are its builder methods: `.n(4)`,
  `.q(Quiet::NoReply)`, `doc(…).detached().scroll(SCROLL_VERTICAL)`,
  `query(1).late()`.

## Dependencies

`hotty` takes flate2 (feature `zlib`, to compress what it sends and inflate
what a relay meets), rustix (feature `term`, for `/dev/tty`) and
unicode-segmentation (grapheme clusters, SDK.md §4.6); base64 and msgpack are
its own. `hotty-ratatui` adds ratatui-core, ratatui-crossterm and
signal-hook.

## Development

`make check` is the gate: rustfmt, the build, the tests (the conformance
vectors among them), clippy, the docs, and the wire layer without default
features. `make vectors` refreshes the vectors from a hotty checkout next to
this one (or `HOTTY_DIR`).

## Licence

Apache-2.0.
