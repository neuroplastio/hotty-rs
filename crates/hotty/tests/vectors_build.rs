//! The `build` vectors: the builders, their defaults and options (SDK.md §3.4).

mod common;
mod wire;

use hotty::*;
use serde_json::Value;
use wire::Out;

fn s<'a>(args: &'a Value, k: &str) -> &'a str {
    args[k].as_str().unwrap_or("")
}

fn u(v: &Value) -> u32 {
    v.as_u64().unwrap_or(0) as u32
}

fn flag(v: &Value) -> Option<bool> {
    v.as_bool()
}

fn placement(p: &Value) -> Placement {
    Placement {
        cols: u(&p["cols"]),
        rows: u(&p["rows"]),
        window: p["window"].as_object().map(|_| Window {
            x: u(&p["window"]["x"]),
            y: u(&p["window"]["y"]),
            w: u(&p["window"]["w"]),
            h: u(&p["window"]["h"]),
        }),
        z: p["z"].as_i64().unwrap_or(0) as i32,
        press: flag(&p["press"]),
        fit: flag(&p["fit"]),
        hover: flag(&p["hover"]),
        keep_cursor: p["keep_cursor"].as_bool().unwrap_or(false),
    }
}

fn op_of(o: &Value) -> Op {
    Op {
        op: s(o, "op").to_string(),
        target: s(o, "target").to_string(),
        key: s(o, "key").to_string(),
        value: o["value"].as_str().map(str::to_string),
    }
}

fn menu_item(i: &Value) -> MenuItem {
    MenuItem {
        key: s(i, "key").to_string(),
        label: s(i, "label").to_string(),
        name: s(i, "name").to_string(),
        enabled: i["enabled"].as_bool().unwrap_or(true),
        items: i["items"]
            .as_array()
            .map(|a| a.iter().map(menu_item).collect())
            .unwrap_or_default(),
    }
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|a| a.iter().map(|x| x.as_str().unwrap().to_string()).collect())
        .unwrap_or_default()
}

fn quiet(v: &Value) -> Option<Quiet> {
    v.as_u64().map(|q| match q {
        0 => Quiet::ReplyAlways,
        1 => Quiet::ReplyOnError,
        2 => Quiet::NoReply,
        _ => panic!("q {q}"),
    })
}

// A Cmd with the reply options, n before q or q before n: a q given wins
// whatever the order (SDK.md §3.4.2).
fn reply_options(mut c: Cmd, o: &Value, q_first: bool) -> Cmd {
    let n = o["n"].as_u64();
    let q = quiet(&o["q"]);
    if q_first && let Some(q) = q {
        c = c.q(q);
    }
    if let Some(n) = n {
        c = c.n(n);
    }
    if !q_first && let Some(q) = q {
        c = c.q(q);
    }
    c
}

fn build(name: &str, a: &Value, o: &Value, q_first: bool) -> String {
    let cmd = |c: Cmd| reply_options(c, o, q_first).to_string();
    match name {
        "query" => {
            let q = query(a["n"].as_u64().unwrap_or(1));
            if o["late"].as_bool() == Some(true) {
                q.late()
            } else {
                q
            }
            .to_string()
        }
        "withdraw_late" => withdraw_late(),
        "doc" => {
            let mut d = doc(s(a, "surface"), s(a, "html"));
            if o["detached"].as_bool() == Some(true) {
                d = d.detached();
            }
            d = d.scroll(u(&o["scroll"]));
            cmd(d.into())
        }
        "place" => cmd(place(s(a, "surface"), &placement(&a["placement"]))),
        "place_at" => cmd(place_at(
            s(a, "surface"),
            u(&a["x"]),
            u(&a["y"]),
            &placement(&a["placement"]),
        )),
        "hide" => cmd(hide(s(a, "surface"))),
        "delta" => {
            let ops: Vec<Op> = a["ops"].as_array().unwrap().iter().map(op_of).collect();
            cmd(delta(s(a, "surface"), &ops))
        }
        "set_text" => cmd(set_text(s(a, "surface"), s(a, "target"), s(a, "text"))),
        "set_var" => cmd(set_var(
            s(a, "surface"),
            s(a, "target"),
            s(a, "name"),
            s(a, "value"),
        )),
        "set_attr" => cmd(set_attr(
            s(a, "surface"),
            s(a, "target"),
            s(a, "name"),
            s(a, "value"),
        )),
        "remove_attr" => cmd(remove_attr(s(a, "surface"), s(a, "target"), s(a, "name"))),
        "morph_to" => cmd(morph_to(s(a, "surface"), s(a, "target"), s(a, "html"))),
        "reveal" => cmd(reveal(s(a, "surface"), s(a, "target"), s(a, "where"))),
        "res" => cmd(res(s(a, "id"), s(a, "mime"), s(a, "data").as_bytes())),
        "del_res" => cmd(del_res(s(a, "id"))),
        "del" => cmd(del(s(a, "surface"))),
        "del_all" => cmd(del_all()),
        "detach" => cmd(detach(s(a, "surface"))),
        "focus" => cmd(focus(s(a, "surface"), s(a, "target"))),
        "blur" => cmd(blur(s(a, "surface"))),
        "edit" => cmd(edit(strs(&a["names"]))),
        "menu" => {
            let tree = a["menu"].as_object().map(|_| MenuTree {
                label: s(&a["menu"], "label").to_string(),
                items: a["menu"]["items"]
                    .as_array()
                    .map(|i| i.iter().map(menu_item).collect())
                    .unwrap_or_default(),
            });
            cmd(menu(tree.as_ref()))
        }
        "menu_show" => cmd(menu_show(strs(&a["keys"]))),
        "menu_hide" => cmd(menu_hide()),
        "touch" => {
            let areas: Vec<TouchArea> = a["areas"]
                .as_array()
                .unwrap()
                .iter()
                .map(|t| TouchArea {
                    c: u(&t["c"]),
                    r: u(&t["r"]),
                    w: u(&t["w"]),
                    h: u(&t["h"]),
                    drag: match s(t, "drag") {
                        "x" => TouchDrag::X,
                        "y" => TouchDrag::Y,
                        "xy" => TouchDrag::Xy,
                        d => panic!("drag {d}"),
                    },
                })
                .collect();
            cmd(touch(&areas))
        }
        "sync" => sync(a["commands"].as_array().unwrap().iter().map(|c| {
            build(
                c["build"].as_str().unwrap(),
                &c["args"],
                &c["options"],
                q_first,
            )
        })),
        other => panic!("no builder {other}"),
    }
}

fn check(v: &Value, out: &str) {
    let got = wire::read(out.as_bytes());
    let want = v["out"].as_array().unwrap();
    assert_eq!(got.len(), want.len(), "pieces of {out:?}: {got:?}");
    for (g, w) in got.iter().zip(want) {
        match (g, w.as_object().unwrap().iter().next().unwrap()) {
            (Out::Raw(b), (k, t)) if k == "raw" => {
                assert_eq!(b, t.as_str().unwrap().as_bytes(), "raw")
            }
            (Out::Cmd(m), (k, c)) if k == "cmd" => {
                assert_eq!(m.control, wire::control_of(&c["control"]), "control");
                assert_eq!(m.payload, wire::payload_of(c), "payload");
            }
            (g, (k, _)) => panic!("got {g:?}, want {k}"),
        }
    }
}

#[test]
fn build_vectors() {
    common::run_all("build", &common::section("build"), |v| {
        let name = v["build"].as_str().unwrap();
        for q_first in [false, true] {
            check(v, &build(name, &v["args"], &v["options"], q_first));
        }
    });
}
