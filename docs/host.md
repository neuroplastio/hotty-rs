---
group: Reference
parent: hotty-rs
order: 30
nav: A host or a relay
path: hotty-rs/host
description: the host's side of the wire in Rust, what hotty-blitz is built on
---

# A host or a relay in Rust

`hotty::host` is the other side of the wire: what a host reads of a
program's output and writes back, and what a relay does that stands between
programs and a host. hotty-blitz, the renderer behind hottyterm and
`hotty run`, is built on it.

```rust
use hotty::host::{Output, Splitter, delta_of, reply_err};

let mut split = Splitter::new();
split.feed(&program_output, &mut |o| match o {
    // To the cells, as they are.
    Output::Bytes(b) => terminal.write(b),
    // The alternate screen, a reset, …: the host acts, then passes on.
    Output::Seq(seq, b) => {
        on_mode(&seq);
        terminal.write(b)
    }
    Output::Command(cmd) => match cmd.action() {
        // Checked whole before any op applies (SPEC §6.1).
        "delta" => match delta_of(&cmd) {
            Ok(ops) => apply(ops),
            // EINVAL, and the index of the op at fault.
            Err(e) => send(reply_err(&cmd, &e)),
        },
        _ => {}
    },
    Output::Invalid(why) => log(why),
});
```

## Reading a program

- **`Splitter`** cuts a program's output, however its reads split it, into
  the bytes to pass on, the control sequences a host acts on (`Seq`:
  private modes such as the alternate screen and synchronized output,
  erase in display, a full reset) with their bytes, and hotty's commands,
  their chunks joined and decoded. `with_max` sets the most a command
  decodes to, which the host reports as `limits.message`.
- **`delta_of`, `field_of`, `menu_of`, `menu_show_of`, `touch_of`,
  `placement_of`** read a command as SPEC has a host read it, and refuse
  one it would refuse, with the `EINVAL` to answer: the detail, and for a
  delta the index of the op at fault. `field_of` checks what needs no
  document; **`control_kind`** and **`field_fits`** say whether the target
  is a control and the change's fields are for it (SPEC §6.4). A
  select's option, `rev` (`ESTALE`) and then the positions are the
  host's: a stale command is `ESTALE` whatever its positions, and one
  that is not is `EINVAL` when a position is negative, past the end, or
  inside a code point.
- **`quiet_of`** says how much of a reply a command asks for, and
  **`speaks`** whether a query lists the version this SDK speaks.

## Writing back

- **`reply_ok`, `reply_err`**: a reply that echoes the command's `n` and
  `s` and names its action, with the keys a reply adds (a placement's `c`
  and `r`) or the error's code and detail.
- **`reply_query`**: the capabilities to a query that lists this version,
  and `EVERSION` to one that doesn't. **`caps_body`** is the capabilities
  alone.
- **`encode_event`**: an event, its detail written from what the
  `hotty::Event` carries for its kind, each field of the type SPEC §3.3
  gives it. A relay that renames an event's surface passes its detail on
  as it came.

Nothing written is compressed: a host never compresses (SPEC §3.3).

## Keys

A host that sees the key itself names a press two ways, by its place and
by what it types (`ctrl+key_w`, `ctrl+w`, SPEC §10.4). `hotty::keys` has
what such a host needs beside what every SDK has: `KeyParts` to take a name
apart and put it together, `char_key`, `Keymap::press` to pick the name a
keymap binds, and `Platform::edit_press` for the edit shortcuts.
