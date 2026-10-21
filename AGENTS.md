# For agents

hotty-rs is the Rust SDK for hotty: the `hotty` crate (the wire layer, a
Field, a Link, a Terminal and a Session) and `hotty-ratatui` (an adapter
for ratatui). The protocol is `SPEC.md` and what an SDK owes is `SDK.md`,
both in neuroplastio/hotty. This repository implements them and must not
grow them.

- **`make check` is the gate**: rustfmt, the build, the tests (the
  conformance vectors among them), clippy with `-D warnings`, the docs with
  no broken links, and the wire layer without default features.
- **The vectors are the contract.** `crates/hotty/tests/testdata/vectors.json`
  is a copy, refreshed with `make vectors`, and a test fails when it falls
  behind the spec's (a hotty checkout next to this repository, or
  `HOTTY_DIR`). One runner per section: `tests/vectors_<section>.rs`, on
  `tests/common`. If the vectors and this SDK disagree, fix the SDK, or
  propose a change to the spec in the hotty repository. Don't quietly
  choose here.
- **The wire layer is pure** (everything in `hotty` but `term`): no I/O, no
  threads, no clock. Time is passed in, in milliseconds. `Link` and
  `Session` are sans I/O too: the I/O is `term`'s, and the adapter's.
- **Canonical names** (SDK.md Appendix A) in Rust's case. Where a name is a
  keyword or clashes, the crate docs (`lib.rs`) say what it became.
- **Protocol only** (SDK.md §2.4). Markdown, charts and document models are
  the program's, in examples if anywhere.
- **Dependencies**: `hotty` takes flate2 (feature `zlib`), rustix (feature
  `term`) and unicode-segmentation, nothing else; base64 and msgpack are its
  own. `hotty-ratatui` adds ratatui-core, ratatui-crossterm and
  signal-hook.
- **hotty-blitz builds on `hotty`** (its host side, the splitter, the
  Decoder and keys), as a path dependency on this checkout. A change to
  `host`, `decoder`, `keys` or the messages runs hotty-blitz's suite too
  (`make test` there, with `HOTTY_DIR`), which holds the host vectors.
- **hotty-ratatui reads the terminal itself.** crossterm's reader takes a
  host's replies for keys, so the session cuts hotty's sequences out and
  parses the rest into crossterm's events (`input.rs`, written from the
  protocols: gov P-7, no code copied from crossterm). Its `Backend` asks for
  the cursor through the session, because ratatui's `Terminal::clear` asks.
- **Felt test on a real host**: xterm-addon-hotty's `serve.py` runs a
  program on a pty in xterm.js with the addon (`python3 serve.py --port
  8791 -- target/debug/examples/board`), and Playwright's Chromium drives
  the page. The unit tests use fake hosts over pipes.
- **Write as the sibling SDKs do**: doc comments say what a thing is and
  does for the program, with the SPEC or SDK.md section; no comments that
  narrate the code.
