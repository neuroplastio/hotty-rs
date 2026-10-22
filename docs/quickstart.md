---
group: Reference
parent: hotty-rs
order: 10
nav: Quick start
path: hotty-rs/quickstart
description: a Rust program that prints HTML in the terminal, and text where it can't
---

# hotty in Rust: a quick start

This page takes a Rust program from nothing to a line of HTML in the
terminal, then to a surface it changes in place. It uses the `hotty` crate
alone: no UI framework. A full-screen program on ratatui reads
[hotty in a ratatui program](ratatui.md) next.

## Install

hotty-rs isn't on crates.io yet. Depend on it from GitHub, and pin a
commit for builds you can reproduce:

```toml
[dependencies]
hotty = { git = "https://github.com/neuroplastio/hotty-rs" }
```

It speaks hotty 0.5. Its `Terminal` is Unix's: it opens `/dev/tty`. It
needs Rust 1.88 or later.

## Hello

```rust
use hotty::{Placement, Terminal};

fn main() -> std::io::Result<()> {
    let Ok(t) = Terminal::open("hello") else {
        println!("Hello."); // no terminal at all: a pipe, a cron job
        return Ok(());
    };
    if !t.detect() {
        println!("Hello. (This terminal is not a hotty host.)");
        return Ok(());
    }
    t.line_start(); // a surface is placed at the cursor: start a line
    t.print(
        "hello",
        "<p>Hello, <b>HTML</b> in the terminal.</p>",
        &Placement::new(40, 1),
    )?;
    t.fence().ok(); // read the host's replies before the shell does
    Ok(())
}
```

Run it in a host: [hottyterm](https://github.com/neuroplastio/hotty/blob/main/docs/hosts.md#hottyterm), or a
page with the browser's host. Anywhere else it prints the text.

What each step does:

- **`Terminal::open`** opens the process's terminal, whatever its standard
  streams are, so `x=$(hello)` still draws on the screen. It fails when
  there is no terminal; then the program writes plain data.
- **`detect`** asks the terminal whether it is a host of hotty 0.5, and
  waits until the answer is in: at once on a host, 150 ms on a terminal
  that answers only its device attributes, 1.5 s at most.
- **`print`** sends the document and places it at the cursor, in one
  write. The document is *detached*: nothing in it reports to the program,
  so it can stay in the scrollback after the program exits.
- **`fence`** waits until the terminal has taken everything written, and
  returns the replies nobody read: the errors, mostly. Read them before
  exiting, or the shell reads them as typing.

## A surface that changes

A program changes a surface it sent with a *delta*: ops that the host
applies together. `var` sets a CSS custom property, the cheap way to move
a bar; `text` replaces an element's text.

```rust
use hotty::{Placement, Terminal, attr, delta, text, var};

const BAR: &str = r#"<style>
  #fill { height: 8px; background: var(--hotty-ansi-4);
          width: calc(var(--p, 0) * 1%) }
</style>
<div id=row><span id=label></span><div id=fill></div></div>"#;

let s = t.print("bar", BAR, &Placement::new(60, 1))?;
for pct in 0..=100 {
    let ops = [var("fill", "p", pct), text("label", format!("{pct}%"))];
    t.send(delta(&s, &ops))?;
    std::thread::sleep(std::time::Duration::from_millis(30));
}
t.send(delta(&s, &[attr("row", "class", "done")]))?;
```

`print` returned the surface's full name: the program's name, its process
id and `bar`, so two runs never share a surface. The
[progress example](https://github.com/neuroplastio/hotty-rs/blob/main/crates/hotty/examples/progress.rs)
is this program, with the line drawn in cells where the terminal is not a
host, and a line a step into a pipe.

## Hearing the user

A surface that is not detached reports what the user does in it. The
events come on the terminal's input, with the keys:

```rust
use hotty::{Placement, doc, place};
use hotty::link::Input;

let s = t.surface("ask");
let form = "<form><input name=who autofocus><button>OK</button></form>";
let at = Placement::new(40, 2);
t.send(format_args!("{}{}", doc(&s, form), place(&s, &at)))?;
while let Some(input) = t.next_input(None) {
    match input {
        Input::Message(m) => {
            if let Some(e) = m.event().filter(|e| e.kind == "submit") {
                println!("{:?}", e.fields);
                break;
            }
        }
        Input::Data(keys) => { /* keys typed outside the surface */ }
        Input::Host(_) => {}
    }
}
t.detach_all()?; // what stays on the screen reports to nobody now
```

## Next

- [hotty in a ratatui program](ratatui.md): surfaces as widgets in a
  full-screen program.
- [A host or a relay](host.md): the other side of the wire.
- The API docs: `cargo doc --open -p hotty`. The builders are named after
  the protocol's actions: `doc`, `place`, `delta`, `set_field`, `hide`, `del`,
  `focus`, `menu`, …, each with SPEC's section.
- [The protocol](https://github.com/neuroplastio/hotty/blob/main/SPEC.md), and
  [how hotty works](https://github.com/neuroplastio/hotty/blob/main/docs/concepts/how-it-works.md).
