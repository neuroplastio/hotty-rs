//! Progress shows a task's progress as a bar that moves in place, and leaves
//! its last state in the scrollback.
//!
//! It shows the cheap way to change a surface many times a second: one delta
//! whose ops move the bar (a custom property) and change the labels (text),
//! a few dozen bytes each, which the host shows together (SPEC §6.1). Where
//! the terminal is not a hotty host, the line is redrawn with a carriage
//! return; into a pipe, a line per step.
//!
//!     cargo run -p hotty --example progress -- 40 50   # files, ms per file

use std::io::{IsTerminal, Write};
use std::thread::sleep;
use std::time::{Duration, Instant};

use hotty::{Placement, Terminal, attr, delta, text, var};

const PAGE: &str = r#"<style>
  .row   { display: flex; align-items: center; gap: 12px; height: 100vh; font: 13px system-ui, sans-serif; }
  .track { flex: 1; height: 8px; border-radius: 4px; background: var(--hotty-ansi-8); overflow: hidden; }
  .fill  { width: calc(var(--p, 0) * 1%); height: 100%; border-radius: 4px; background: var(--hotty-ansi-4); }
  .done .fill { background: var(--hotty-ansi-2); }
  #pct   { width: 4ch; text-align: right; font-variant-numeric: tabular-nums; }
  #label { width: 24ch; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
</style>
<div class="row" id="row"><span id="label">starting</span><div class="track"><div class="fill" id="fill"></div></div><span id="pct">0%</span></div>"#;

/// How a rendition shows the work: each step, then the end.
trait View {
    fn step(&mut self, done: usize, total: usize, name: &str);
    fn end(&mut self, total: usize, took: Duration);
}

/// The surface: printed once, then changed by deltas.
struct Bar {
    t: Terminal,
    s: String,
}

impl View for Bar {
    fn step(&mut self, done: usize, total: usize, name: &str) {
        let pct = done * 100 / total;
        let _ = self.t.send(delta(
            &self.s,
            &[
                var("fill", "p", pct),
                text("label", name),
                text("pct", format!("{pct}%")),
            ],
        ));
    }

    fn end(&mut self, total: usize, took: Duration) {
        let _ = self.t.send(delta(
            &self.s,
            &[
                text("label", format!("{total} files in {} ms", took.as_millis())),
                attr("row", "class", "row done"),
            ],
        ));
        // Read the host's replies before the shell does.
        let _ = self.t.fence();
    }
}

/// The line redrawn in place.
struct Cells;

impl View for Cells {
    fn step(&mut self, done: usize, total: usize, name: &str) {
        let n = done * 30 / total;
        print!(
            "\r\x1b[K[{}{}] {:>3}% {name}",
            "#".repeat(n),
            ".".repeat(30 - n),
            done * 100 / total
        );
        let _ = std::io::stdout().flush();
    }

    fn end(&mut self, total: usize, took: Duration) {
        println!("\r\x1b[K✓ {total} files in {} ms", took.as_millis());
    }
}

/// A line a step, for a pipe.
struct Lines;

impl View for Lines {
    fn step(&mut self, _done: usize, _total: usize, name: &str) {
        println!("{name}");
    }

    fn end(&mut self, _total: usize, _took: Duration) {}
}

fn view() -> Box<dyn View> {
    if !std::io::stdout().is_terminal() {
        return Box::new(Lines);
    }
    match Terminal::open("progress") {
        Ok(t) if t.detect() => {
            t.line_start();
            let cols = u32::from(t.size().0).min(72);
            // Detached from the start: the program reads nothing from it, and
            // it stays in the scrollback after the program exits (SPEC §5.5).
            match t.print("bar", PAGE, &Placement::new(cols, 1)) {
                Ok(s) => Box::new(Bar { t, s }),
                Err(_) => Box::new(Cells),
            }
        }
        _ => Box::new(Cells),
    }
}

fn main() {
    let mut args = std::env::args().skip(1).map(|a| a.parse().unwrap_or(0));
    let files = args.next().filter(|&n| n > 0).unwrap_or(40);
    let delay = Duration::from_millis(args.next().unwrap_or(50) as u64);
    let mut v = view();
    let start = Instant::now();
    for i in 1..=files {
        sleep(delay);
        v.step(i, files, &format!("src/file{i:03}.rs"));
    }
    v.end(files, start.elapsed());
}
