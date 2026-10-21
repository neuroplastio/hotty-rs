//! Hello prints a line of HTML in the terminal, and a line of text where the
//! terminal cannot show HTML.
//!
//! It is the smallest hotty program: open the terminal, ask whether it is a
//! host, print a document at the cursor, and read the host's replies before
//! exiting, so that none is left for the shell.
//!
//!     cargo run -p hotty --example hello

use hotty::{Placement, Terminal};

fn main() -> std::io::Result<()> {
    let Ok(t) = Terminal::open("hello") else {
        println!("Hello."); // no terminal at all: a pipe, a cron job
        return Ok(());
    };
    if !t.detect() {
        println!("Hello. (This terminal is not a hotty host, so this is text.)");
        return Ok(());
    }
    // Surfaces are placed at the cursor: start on a line of their own.
    t.line_start();
    // The host's stylesheet gives every document the terminal's colours and
    // font (SPEC §8). print sends the document detached: nothing in it
    // reports, so nothing is left for the shell to read.
    t.print(
        "hello",
        r#"<p style="margin: 0">Hello, <b style="color: var(--hotty-ansi-5)">HTML</b> in the terminal.</p>"#,
        &Placement::new(40, 1),
    )?;
    // The replies to what it sent: errors, if any.
    match t.fence() {
        Ok(replies) => {
            for r in replies.iter().filter_map(|r| r.err()) {
                eprintln!("{r}");
            }
        }
        Err(e) => eprintln!("{e}"),
    }
    Ok(())
}
