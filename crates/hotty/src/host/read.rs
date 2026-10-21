//! A program's commands, as a host or a relay reads them.

use crate::commands::{
    MENU_DEPTH, MenuItem, MenuTree, Op, Placement, Quiet, TOUCH_MAX, TouchArea, TouchDrag, Window,
};
use crate::messages::{Error, Message};
use crate::msgpack::{self, Value};
use crate::{MAX_SIZE, OPS, VERSION, code};

/// The `EINVAL` a host answers a command with: what was wrong, and the op
/// at fault in a delta.
fn invalid(m: &Message, detail: impl Into<String>, op: Option<usize>) -> Error {
    Error {
        code: code::EINVAL.to_string(),
        detail: detail.into(),
        re: m.action().to_string(),
        surface: m.get("s").map(str::to_string),
        op: op.and_then(|i| u32::try_from(i).ok()),
    }
}

// The body as one msgpack map, read strictly (SPEC §3.3, SDK.md §3.9).
fn map_of(m: &Message) -> Option<Value> {
    msgpack::read(&m.payload).filter(|v| matches!(v, Value::Map(_)))
}

// A str field: absent is empty, another type an error.
fn text(of: &Value, name: &str) -> Result<String, String> {
    match of.get(name) {
        None => Ok(String::new()),
        Some(Value::Str(s)) => Ok(s.clone()),
        Some(_) => Err(format!("{name} is not a str")),
    }
}

// An array field: absent is empty, another type an error.
fn list<'a>(of: &'a Value, name: &str) -> Result<&'a [Value], String> {
    match of.get(name) {
        None => Ok(&[]),
        Some(Value::Array(a)) => Ok(a),
        Some(_) => Err(format!("{name} is not an array")),
    }
}

/// The quiet level a command asks for (SPEC §3.5): its `q` when that is 0,
/// 1 or 2, and otherwise what the wire implies: 0 for a query and for a
/// command with `n`, 1 for any other.
pub fn quiet_of(m: &Message) -> Quiet {
    match m.get("q") {
        Some("0") => Quiet::ReplyAlways,
        Some("1") => Quiet::ReplyOnError,
        Some("2") => Quiet::NoReply,
        _ if m.action() == "q" || m.get("n").is_some() => Quiet::ReplyAlways,
        _ => Quiet::ReplyOnError,
    }
}

/// Whether a query lists [`VERSION`] among the versions the program speaks
/// (`v`, comma-separated, SPEC §4). A host that speaks only this version
/// answers such a query with its capabilities, and any other with
/// `EVERSION` ([`reply_query`](crate::host::reply_query)).
pub fn speaks(query: &Message) -> bool {
    query
        .get("v")
        .is_some_and(|v| v.split(',').any(|v| v == VERSION))
}

/// Reads a delta's ops (SPEC §6.1), checking the whole body before any op
/// applies. It is `EINVAL`, with [`Error::op`] the index of the first op at
/// fault where one is, when the body does not decode or is not `{"ops":
/// [op]}` with at least one op; an op is not a map, holds `op`, `t`, `k` or
/// `v` as anything but a str, or names an op the table does not have; an op
/// lacks the `t` it needs (every op but `morph`, whose empty `t` is none) or
/// the `k` of `attr`, `unattr` and `var`; or `reveal` has a `v` other than
/// `start` or `end`. A `v` an op does not take is dropped, and so is a key
/// the table does not name.
pub fn delta_of(m: &Message) -> Result<Vec<Op>, Error> {
    let Some(root) = map_of(m) else {
        return Err(invalid(m, "the delta's body does not decode", None));
    };
    let ops = match root.get("ops") {
        Some(Value::Array(a)) if !a.is_empty() => a,
        Some(Value::Array(_)) => return Err(invalid(m, "the delta has no ops", None)),
        _ => return Err(invalid(m, "the delta's body is not {\"ops\": [op]}", None)),
    };
    let mut out = Vec::with_capacity(ops.len());
    for (i, o) in ops.iter().enumerate() {
        let at = |detail: String| invalid(m, detail, Some(i));
        if !matches!(o, Value::Map(_)) {
            return Err(at(format!("op {i} is not a map")));
        }
        let field = |name: &str| match o.get(name) {
            None => Ok(None),
            Some(Value::Str(s)) => Ok(Some(s.clone())),
            Some(_) => Err(at(format!("op {i}: {name} is not a str"))),
        };
        let name = field("op")?.ok_or_else(|| at(format!("op {i} names no op")))?;
        if !OPS.contains(&name.as_str()) {
            return Err(at(format!("op {i}: unknown op {name}")));
        }
        let (t, k, v) = (field("t")?, field("k")?, field("v")?);
        if name != "morph" && t.as_deref().is_none_or(str::is_empty) {
            return Err(at(format!("op {i}: {name} needs t")));
        }
        if matches!(name.as_str(), "attr" | "unattr" | "var")
            && k.as_deref().is_none_or(str::is_empty)
        {
            return Err(at(format!("op {i}: {name} needs k")));
        }
        let v = v.filter(|_| !matches!(name.as_str(), "remove" | "unattr"));
        if name == "reveal" && v.as_deref().is_some_and(|v| !matches!(v, "start" | "end")) {
            return Err(at(format!("op {i}: reveal takes start, end or nothing")));
        }
        out.push(Op {
            op: name,
            target: t.unwrap_or_default(),
            key: k.unwrap_or_default(),
            value: v,
        });
    }
    Ok(out)
}

/// Reads a menu (SPEC §10.7): `None` for a command with no body, which
/// leaves the program no menu. Every field of the tree is filled: empty
/// strs, `enabled` false where an item has none, and no items for an
/// action. A body that is no menu, holds a field of another type, or nests
/// its items deeper than [`MENU_DEPTH`] is `EINVAL`. A command with `show`
/// is not a menu: [`menu_show_of`] reads it.
pub fn menu_of(m: &Message) -> Result<Option<MenuTree>, Error> {
    if m.payload.is_empty() {
        return Ok(None);
    }
    let Some(root) = map_of(m) else {
        return Err(invalid(m, "the menu's body does not decode", None));
    };
    fn items(of: &Value, depth: usize) -> Result<Vec<MenuItem>, String> {
        let list = list(of, "items")?;
        if depth > MENU_DEPTH && !list.is_empty() {
            return Err(format!("items nest more than {MENU_DEPTH} deep"));
        }
        list.iter()
            .map(|i| {
                if !matches!(i, Value::Map(_)) {
                    return Err("an item is not a map".to_string());
                }
                Ok(MenuItem {
                    key: text(i, "key")?,
                    label: text(i, "label")?,
                    name: text(i, "name")?,
                    enabled: match i.get("enabled") {
                        None => false,
                        Some(Value::Bool(b)) => *b,
                        Some(_) => return Err("enabled is not a bool".to_string()),
                    },
                    items: items(i, depth + 1)?,
                })
            })
            .collect()
    }
    let tree = (|| {
        Ok::<_, String>(MenuTree {
            label: text(&root, "label")?,
            items: items(&root, 1)?,
        })
    })();
    tree.map(Some).map_err(|e| invalid(m, e, None))
}

/// Reads `a=menu:show=1`'s keys (SPEC §10.7): the keys typed, as the program
/// names them, whose level to show. No body, or no `seq` in it, is the top
/// level: none. A body that is no map, or whose `seq` is not an array of
/// strs, is `EINVAL`.
pub fn menu_show_of(m: &Message) -> Result<Vec<String>, Error> {
    if m.payload.is_empty() {
        return Ok(Vec::new());
    }
    let Some(root) = map_of(m) else {
        return Err(invalid(m, "the body does not decode", None));
    };
    list(&root, "seq")
        .and_then(|seq| {
            seq.iter()
                .map(|k| {
                    k.as_str()
                        .map(str::to_string)
                        .ok_or_else(|| "seq holds a non-str".to_string())
                })
                .collect()
        })
        .map_err(|e| invalid(m, e, None))
}

/// Reads `a=touch`'s areas (SPEC §9.6): none for a command with no body. A
/// body that is not `{"areas": [area]}`, an area without one of `c`, `r`,
/// `w`, `h` and `drag` or with one of another type (an int sent as a float
/// among them), `c` or `r` under 0, `w` or `h` under 1, a `drag` other than
/// `x`, `y` and `xy`, or more than [`TOUCH_MAX`] areas is `EINVAL`.
pub fn touch_of(m: &Message) -> Result<Vec<TouchArea>, Error> {
    if m.payload.is_empty() {
        return Ok(Vec::new());
    }
    let Some(root) = map_of(m) else {
        return Err(invalid(m, "the body does not decode", None));
    };
    let areas = match root.get("areas") {
        Some(Value::Array(a)) => a,
        Some(_) => return Err(invalid(m, "areas is not an array", None)),
        None => return Err(invalid(m, "the body is not {\"areas\": [area]}", None)),
    };
    if areas.len() > TOUCH_MAX {
        let n = areas.len();
        return Err(invalid(
            m,
            format!("{n} areas, more than {TOUCH_MAX}"),
            None,
        ));
    }
    areas
        .iter()
        .enumerate()
        .map(|(i, a)| {
            if !matches!(a, Value::Map(_)) {
                return Err(format!("area {i} is not a map"));
            }
            let int = |name: &str, least: i64| match a.get(name) {
                Some(Value::Int(v)) if *v >= least => Ok(u32::try_from(*v).unwrap_or(u32::MAX)),
                Some(Value::Int(_)) => Err(format!("area {i}: {name} is under {least}")),
                Some(_) => Err(format!("area {i}: {name} is not an int")),
                None => Err(format!("area {i} has no {name}")),
            };
            let drag = match a.get("drag") {
                Some(Value::Str(d)) => match d.as_str() {
                    "x" => TouchDrag::X,
                    "y" => TouchDrag::Y,
                    "xy" => TouchDrag::Xy,
                    _ => return Err(format!("area {i}: drag {d:?} is not x, y or xy")),
                },
                Some(_) => return Err(format!("area {i}: drag is not a str")),
                None => return Err(format!("area {i} has no drag")),
            };
            Ok(TouchArea {
                c: int("c", 0)?,
                r: int("r", 0)?,
                w: int("w", 1)?,
                h: int("h", 1)?,
                drag,
            })
        })
        .collect::<Result<_, _>>()
        .map_err(|e| invalid(m, e, None))
}

/// Reads a placement (SPEC §5.2): `rows` 0 for `r=auto` or none, and the
/// window as the command gives it, its defaults filled in: `w` reaches the
/// surface's right edge, and `h` its bottom, which with `rows` 0 is the
/// host's to find (`h` 0). A placement that leaves `press`, `fit` or `hover`
/// out, or gives one a value other than `0` or `1`, has it `None`. A command
/// without `c`, or with a size, a window or a `z` out of range, is
/// `EINVAL`.
pub fn placement_of(m: &Message) -> Result<Placement, Error> {
    let num = |k: &str, default: i64, lo: i64, hi: i64| -> Option<i64> {
        match m.get(k) {
            None => Some(default),
            Some(v) => v.parse::<i64>().ok().filter(|n| (lo..=hi).contains(n)),
        }
    };
    let max = i64::from(MAX_SIZE);
    if m.get("c").is_none() {
        return Err(invalid(m, "place needs c=<cols>", None));
    }
    let cols = num("c", 0, 1, max).ok_or_else(|| invalid(m, "c out of range", None))?;
    let rows = match m.get("r") {
        None | Some("auto") => 0,
        Some(_) => num("r", 0, 1, max).ok_or_else(|| invalid(m, "r out of range", None))?,
    };
    let window = if ["x", "y", "w", "h"].iter().any(|k| m.get(k).is_some()) {
        // A default is not range-checked: with auto rows, an h left out is
        // 0, the host's to find.
        let x = num("x", 0, 0, max - 1);
        let y = num("y", 0, 0, max - 1);
        let w = x.and_then(|x| num("w", cols - x, 1, max));
        let h = y.and_then(|y| num("h", (rows - y).max(0), 1, max));
        match (x, y, w, h) {
            (Some(x), Some(y), Some(w), Some(h))
                if w >= 1 && x + w <= cols && (rows == 0 || (h >= 1 && y + h <= rows)) =>
            {
                Some(Window {
                    x: x as u32,
                    y: y as u32,
                    w: w as u32,
                    h: h as u32,
                })
            }
            _ => return Err(invalid(m, "the window is not inside the surface", None)),
        }
    } else {
        None
    };
    let z = num("z", 0, -1000, 1000)
        .ok_or_else(|| invalid(m, "z is an integer from -1000 to 1000", None))?;
    let flag = |k: &str| match m.get(k) {
        Some("1") => Some(true),
        Some("0") => Some(false),
        _ => None,
    };
    Ok(Placement {
        cols: cols as u32,
        rows: rows as u32,
        window,
        z: z as i32,
        press: flag("p"),
        fit: flag("f"),
        hover: flag("v"),
        keep_cursor: m.get("C") == Some("1"),
    })
}

/// The cursor's move a placement without `C=1` makes (SPEC §5.2): `rows`
/// index operations (IND) and a carriage return, to the start of the line
/// below the placement. A relay that keeps a placement from the screen it
/// emulates feeds it this instead.
pub fn cursor_below(rows: u32) -> String {
    let mut s = "\x1bD".repeat(rows as usize);
    s.push('\r');
    s
}
