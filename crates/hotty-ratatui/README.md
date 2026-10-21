# hotty-ratatui

hotty (HTML over the TTY) for [ratatui](https://ratatui.rs) programs: a
`Surface` widget renders an HTML document where the terminal is a hotty
host, and its fallback widget where it is not.

```rust
let (mut terminal, session) = hotty_ratatui::init("app")?;
let card = session.surface("card");
session.draw(&mut terminal, |frame, surfaces| {
    frame.render_stateful_widget(
        Surface::new(&card, || "<p>Hello, <b>HTML</b></p>".into())
            .fallback(Paragraph::new("Hello, cells")),
        frame.area(),
        surfaces,
    );
})?;
match session.read()? {
    Event::Input(ev) => { /* crossterm's event, for the handler you have */ }
    Event::Hotty(ev) => { /* a click, a submit, … in a surface */ }
    _ => {}
}
```

The session reads the terminal itself, since crossterm's reader would take
the host's replies for keys: read events with `session.read()` and
`session.poll()` in place of crossterm's, and nothing else may read the
terminal while the session is open. Unix only.

See the [repository's README](https://github.com/neuroplastio/hotty-rs) for
the whole of it.
