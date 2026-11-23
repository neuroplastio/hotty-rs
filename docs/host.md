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
  inside a code point. `placement_of` reads `b=1`, and only `1`, as a
  backdrop (SPEC §5.2): a host without the feature `backdrop` ignores it,
  and a relay that places the surface again with `place` passes it on, as
  it does `z`.
- **`hit_of`** reads a hit query into `Hit { c, r }`, signed decimal cells
  counted from the surface's top left, or returns `EINVAL` (SPEC §9.7).
  The host checks `ENOENT` and `EDETACHED` against its surfaces. `quiet_of`
  treats `hit` as a query, implying level 0 with or without `n`.
- **`quiet_of`** says how much of a reply a command asks for, and
  **`answer_in`** the version a query is answered in, the highest of
  `hotty::VERSIONS` (0.8 and 0.7) it lists, or `None`; **`speaks`** whether
  it lists one.

## Writing back

- **`reply_ok`, `reply_err`**: a reply that echoes the command's `n` and
  `s` and names its action, with the keys a reply adds (a placement's `c`
  and `r`) or the error's code and detail.
- **`reply_query`**: the capabilities to a query that lists a version of
  `VERSIONS`, with `v` set to `answer_in`'s (a host that speaks 0.7 too keeps
  that session as 0.7 has it, HOST.md §5), and `EVERSION` to one that
  doesn't, its detail `0.7,0.8`. **`caps_body`** is the capabilities
  alone.
- **`reply_hit`** answers a hit query with a `hotty::HitCell`: the target
  `t`, empty for none, and the steps `tx` and `ty` where it has them (SPEC
  §9.7). **`hit_body`** writes that body alone, omitting absent fields.
- **`encode_event`** writes the detail for the event kind (SPEC §3.3,
  §9). In a 0.9 session, send a primary `press`, a `contextmenu`, or an
  `auxclick` as the gesture calls for. Put the button on a non-primary
  `dragstart`. In a 0.8 session, send the menu gesture as a `press` with
  `BUTTON_SECONDARY` and its cell (HOST.md §5.2). A relay that renames an
  event's surface passes its detail on as it came.

Nothing written is compressed: a host never compresses (SPEC §3.3).

## Keys

A host that sees the key itself names a press two ways, by its place and
by what it types (`ctrl+key_w`, `ctrl+w`, SPEC §10.4). `hotty::keys` has
what such a host needs beside what every SDK has: `KeyParts` to take a name
apart and put it together, `char_key`, `Keymap::press` to pick the name a
keymap binds, and `Platform::edit_press` for the edit shortcuts.

`parse_keymap` keeps `tab=program` and `tab=default`, but `Keymap::program`
gives Tab to the program only on a field's keymap, from `resolve` (SPEC
§10.2). A binding to `default` restores the default field action or typing;
outside a field it takes a farther binding back, so the host uses the key
as with no keymap.
