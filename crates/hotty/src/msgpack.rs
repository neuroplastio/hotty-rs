//! msgpack, as hotty's bodies use it (SPEC §3.3): written in the smallest
//! of each type's forms, and read strictly (SDK.md §3.9).

/// Writes msgpack values, each in the smallest of its type's forms.
#[derive(Debug, Default)]
pub(crate) struct Writer {
    pub(crate) out: Vec<u8>,
}

impl Writer {
    pub(crate) fn new() -> Writer {
        Writer::default()
    }

    pub(crate) fn finish(self) -> Vec<u8> {
        self.out
    }

    fn len(&mut self, n: usize, fix: u8, fix_max: usize, m8: Option<u8>, m16: u8, m32: u8) {
        if n <= fix_max {
            self.out.push(fix | n as u8);
        } else if let (Some(m8), true) = (m8, n <= 0xff) {
            self.out.extend([m8, n as u8]);
        } else if n <= 0xffff {
            self.out.push(m16);
            self.out.extend((n as u16).to_be_bytes());
        } else {
            self.out.push(m32);
            self.out.extend((n as u32).to_be_bytes());
        }
    }

    pub(crate) fn map(&mut self, n: usize) {
        self.len(n, 0x80, 15, None, 0xde, 0xdf);
    }

    pub(crate) fn array(&mut self, n: usize) {
        self.len(n, 0x90, 15, None, 0xdc, 0xdd);
    }

    pub(crate) fn str(&mut self, s: &str) {
        self.len(s.len(), 0xa0, 31, Some(0xd9), 0xda, 0xdb);
        self.out.extend(s.as_bytes());
    }

    pub(crate) fn bool(&mut self, b: bool) {
        self.out.push(if b { 0xc3 } else { 0xc2 });
    }

    pub(crate) fn int(&mut self, v: i64) {
        if v >= 0 {
            let u = v as u64;
            if u < 0x80 {
                self.out.push(u as u8);
            } else if u <= 0xff {
                self.out.extend([0xcc, u as u8]);
            } else if u <= 0xffff {
                self.out.push(0xcd);
                self.out.extend((u as u16).to_be_bytes());
            } else if u <= 0xffff_ffff {
                self.out.push(0xce);
                self.out.extend((u as u32).to_be_bytes());
            } else {
                self.out.push(0xcf);
                self.out.extend(u.to_be_bytes());
            }
        } else if v >= -32 {
            self.out.push(v as i8 as u8);
        } else if v >= i64::from(i8::MIN) {
            self.out.extend([0xd0, v as i8 as u8]);
        } else if v >= i64::from(i16::MIN) {
            self.out.push(0xd1);
            self.out.extend((v as i16).to_be_bytes());
        } else if v >= i64::from(i32::MIN) {
            self.out.push(0xd2);
            self.out.extend((v as i32).to_be_bytes());
        } else {
            self.out.push(0xd3);
            self.out.extend(v.to_be_bytes());
        }
    }

    #[cfg(test)]
    pub(crate) fn float(&mut self, v: f64) {
        self.out.push(0xcb);
        self.out.extend(v.to_be_bytes());
    }

    /// A key and a str value.
    pub(crate) fn kv_str(&mut self, k: &str, v: &str) {
        self.str(k);
        self.str(v);
    }
}

/// How deep a body may nest, its map being the first level (SDK.md §3.9).
pub(crate) const MAX_DEPTH: usize = 32;

/// The largest int a body holds, either way from zero (SPEC §3.3): what a
/// double holds exactly.
pub(crate) const MAX_INT: i64 = (1 << 53) - 1;

/// One msgpack value, as [`read`] gives it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Value {
    Bool(bool),
    Int(i64),
    /// A float of 32 or 64 bits.
    Float(f64),
    Str(String),
    Bin(Vec<u8>),
    Array(Vec<Value>),
    /// A map, in its order: its keys are strings, each given once.
    Map(Vec<(String, Value)>),
    /// A timestamp (extension −1): seconds since 1970, and nanoseconds into
    /// the second.
    Time {
        secs: i64,
        nanos: u32,
    },
    /// An extension of another type, which no field of hotty carries.
    Ext(i8, Vec<u8>),
}

impl Value {
    /// The map's value for `key`.
    pub(crate) fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Map(m) => m.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub(crate) fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub(crate) fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub(crate) fn as_float(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            _ => None,
        }
    }

    pub(crate) fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub(crate) fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    pub(crate) fn as_map(&self) -> Option<&[(String, Value)]> {
        match self {
            Value::Map(m) => Some(m),
            _ => None,
        }
    }
}

/// The one msgpack value the bytes hold, or `None` when they do not hold
/// exactly one: cut short, followed by more, or starting with 0xc1. Nor do
/// they when they hold, anywhere, what no host sends (SPEC §3.3, SDK.md
/// §3.9): a nil, a map key that is not a str or is given twice in one map,
/// a str that is not UTF-8, an int further than 2^53 − 1 from zero, or a
/// timestamp of another size than 4, 8 or 12 bytes, with a second's
/// nanoseconds or more, or with seconds further than 2^53 − 1 from 1970;
/// or when they nest more than [`MAX_DEPTH`] deep.
pub(crate) fn read(bytes: &[u8]) -> Option<Value> {
    let mut r = Reader { b: bytes, i: 0 };
    let v = r.value(0)?;
    (r.i == bytes.len()).then_some(v)
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.i.checked_add(n).filter(|&e| e <= self.b.len())?;
        let s = &self.b[self.i..end];
        self.i = end;
        Some(s)
    }

    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }

    fn be<const N: usize>(&mut self) -> Option<[u8; N]> {
        self.take(N)?.try_into().ok()
    }

    fn len16(&mut self) -> Option<usize> {
        Some(u16::from_be_bytes(self.be()?) as usize)
    }

    fn len32(&mut self) -> Option<usize> {
        Some(u32::from_be_bytes(self.be()?) as usize)
    }

    fn int(x: i128) -> Option<Value> {
        (x.abs() <= i128::from(MAX_INT)).then_some(Value::Int(x as i64))
    }

    fn str(&mut self, n: usize) -> Option<Value> {
        let s = std::str::from_utf8(self.take(n)?).ok()?;
        Some(Value::Str(s.to_owned()))
    }

    fn bin(&mut self, n: usize) -> Option<Value> {
        Some(Value::Bin(self.take(n)?.to_vec()))
    }

    fn ext(&mut self, n: usize) -> Option<Value> {
        let t = self.u8()? as i8;
        let d = self.take(n)?;
        if t != -1 {
            return Some(Value::Ext(t, d.to_vec()));
        }
        let (secs, nanos) = match n {
            4 => (i64::from(u32::from_be_bytes(d.try_into().ok()?)), 0),
            8 => {
                let x = u64::from_be_bytes(d.try_into().ok()?);
                ((x & ((1 << 34) - 1)) as i64, (x >> 34) as u32)
            }
            12 => {
                let nanos = u32::from_be_bytes(d[..4].try_into().ok()?);
                let secs = i64::from_be_bytes(d[4..].try_into().ok()?);
                if secs.unsigned_abs() > MAX_INT as u64 {
                    return None;
                }
                (secs, nanos)
            }
            _ => return None,
        };
        (nanos < 1_000_000_000).then_some(Value::Time { secs, nanos })
    }

    // A container of n entries at depth, each at least `each` bytes: a
    // length past the bytes left is cut short, and nothing that long is made.
    fn open(&self, depth: usize, n: usize, each: usize) -> Option<usize> {
        if depth >= MAX_DEPTH || n.checked_mul(each)? > self.b.len() - self.i {
            return None;
        }
        Some(n)
    }

    fn array(&mut self, depth: usize, n: usize) -> Option<Value> {
        let n = self.open(depth, n, 1)?;
        let mut a = Vec::with_capacity(n);
        for _ in 0..n {
            a.push(self.value(depth + 1)?);
        }
        Some(Value::Array(a))
    }

    fn map(&mut self, depth: usize, n: usize) -> Option<Value> {
        let n = self.open(depth, n, 2)?;
        let mut m: Vec<(String, Value)> = Vec::with_capacity(n);
        let mut given = std::collections::HashSet::new();
        for _ in 0..n {
            let Value::Str(k) = self.value(depth + 1)? else {
                return None;
            };
            if !given.insert(k.clone()) {
                return None;
            }
            let v = self.value(depth + 1)?;
            m.push((k, v));
        }
        Some(Value::Map(m))
    }

    fn value(&mut self, depth: usize) -> Option<Value> {
        let c = self.u8()?;
        match c {
            0x00..=0x7f => Some(Value::Int(i64::from(c))),
            0x80..=0x8f => self.map(depth, usize::from(c & 0x0f)),
            0x90..=0x9f => self.array(depth, usize::from(c & 0x0f)),
            0xa0..=0xbf => self.str(usize::from(c & 0x1f)),
            0xc2 => Some(Value::Bool(false)),
            0xc3 => Some(Value::Bool(true)),
            0xc4 => {
                let n = self.u8()?;
                self.bin(usize::from(n))
            }
            0xc5 => {
                let n = self.len16()?;
                self.bin(n)
            }
            0xc6 => {
                let n = self.len32()?;
                self.bin(n)
            }
            0xc7 => {
                let n = self.u8()?;
                self.ext(usize::from(n))
            }
            0xc8 => {
                let n = self.len16()?;
                self.ext(n)
            }
            0xc9 => {
                let n = self.len32()?;
                self.ext(n)
            }
            0xca => Some(Value::Float(f64::from(f32::from_be_bytes(self.be()?)))),
            0xcb => Some(Value::Float(f64::from_be_bytes(self.be()?))),
            0xcc => Some(Value::Int(i64::from(self.u8()?))),
            0xcd => Some(Value::Int(i64::from(u16::from_be_bytes(self.be()?)))),
            0xce => Some(Value::Int(i64::from(u32::from_be_bytes(self.be()?)))),
            0xcf => Self::int(i128::from(u64::from_be_bytes(self.be()?))),
            0xd0 => Some(Value::Int(i64::from(self.u8()? as i8))),
            0xd1 => Some(Value::Int(i64::from(i16::from_be_bytes(self.be()?)))),
            0xd2 => Some(Value::Int(i64::from(i32::from_be_bytes(self.be()?)))),
            0xd3 => Self::int(i128::from(i64::from_be_bytes(self.be()?))),
            0xd4 => self.ext(1),
            0xd5 => self.ext(2),
            0xd6 => self.ext(4),
            0xd7 => self.ext(8),
            0xd8 => self.ext(16),
            0xd9 => {
                let n = self.u8()?;
                self.str(usize::from(n))
            }
            0xda => {
                let n = self.len16()?;
                self.str(n)
            }
            0xdb => {
                let n = self.len32()?;
                self.str(n)
            }
            0xdc => {
                let n = self.len16()?;
                self.array(depth, n)
            }
            0xdd => {
                let n = self.len32()?;
                self.array(depth, n)
            }
            0xde => {
                let n = self.len16()?;
                self.map(depth, n)
            }
            0xdf => {
                let n = self.len32()?;
                self.map(depth, n)
            }
            0xe0..=0xff => Some(Value::Int(i64::from(c as i8))),
            // 0xc0, a nil, which no host sends, and 0xc1, which is no type.
            0xc0 | 0xc1 => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(f: impl FnOnce(&mut Writer)) -> Vec<u8> {
        let mut w = Writer::new();
        f(&mut w);
        w.finish()
    }

    #[test]
    fn writes_the_smallest_forms() {
        assert_eq!(w(|w| w.int(0)), [0x00]);
        assert_eq!(w(|w| w.int(127)), [0x7f]);
        assert_eq!(w(|w| w.int(128)), [0xcc, 0x80]);
        assert_eq!(w(|w| w.int(256)), [0xcd, 0x01, 0x00]);
        assert_eq!(w(|w| w.int(65536)), [0xce, 0, 1, 0, 0]);
        assert_eq!(w(|w| w.int(-1)), [0xff]);
        assert_eq!(w(|w| w.int(-32)), [0xe0]);
        assert_eq!(w(|w| w.int(-33)), [0xd0, 0xdf]);
        assert_eq!(w(|w| w.int(-129)), [0xd1, 0xff, 0x7f]);
        assert_eq!(w(|w| w.str("")), [0xa0]);
        assert_eq!(w(|w| w.str(&"x".repeat(31)))[0], 0xbf);
        assert_eq!(w(|w| w.str(&"x".repeat(32)))[..2], [0xd9, 32]);
        assert_eq!(w(|w| w.str(&"x".repeat(256)))[..3], [0xda, 1, 0]);
        assert_eq!(w(|w| w.map(15)), [0x8f]);
        assert_eq!(w(|w| w.map(16)), [0xde, 0, 16]);
        assert_eq!(w(|w| w.array(1)), [0x91]);
        assert_eq!(w(|w| w.bool(true)), [0xc3]);
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn reads_what_it_writes() {
        let b = w(|w| {
            w.map(4);
            w.kv_str("a", "é");
            w.str("n");
            w.int(-300);
            w.str("l");
            w.array(2);
            w.bool(false);
            w.float(1.5);
            w.str("big");
            w.int(MAX_INT);
        });
        assert_eq!(
            read(&b),
            Some(Value::Map(vec![
                ("a".into(), Value::Str("é".into())),
                ("n".into(), Value::Int(-300)),
                (
                    "l".into(),
                    Value::Array(vec![Value::Bool(false), Value::Float(1.5)])
                ),
                ("big".into(), Value::Int(MAX_INT)),
            ]))
        );
    }

    #[test]
    fn reads_every_form_of_a_type() {
        assert_eq!(read(&hex("cd0007")), Some(Value::Int(7)));
        assert_eq!(read(&hex("d1fffe")), Some(Value::Int(-2)));
        assert_eq!(read(&hex("ca43a04000")), Some(Value::Float(320.5)));
        assert_eq!(read(&hex("da00026162")), Some(Value::Str("ab".into())));
        assert_eq!(
            read(&hex("dc0001c3")),
            Some(Value::Array(vec![Value::Bool(true)]))
        );
        assert_eq!(
            read(&hex("de0001a16101")).unwrap().get("a"),
            Some(&Value::Int(1))
        );
        assert_eq!(read(&hex("c40278ff")), Some(Value::Bin(vec![0x78, 0xff])));
        assert_eq!(read(&hex("d40501")), Some(Value::Ext(5, vec![1])));
    }

    #[test]
    fn reads_timestamps_of_4_8_and_12_bytes() {
        assert_eq!(
            read(&hex("d6ff00000010")),
            Some(Value::Time { secs: 16, nanos: 0 })
        );
        // 1 ns and 2 s: 30 bits of nanoseconds, then 34 of seconds.
        assert_eq!(
            read(&hex("d7ff0000000400000002")),
            Some(Value::Time { secs: 2, nanos: 1 })
        );
        assert_eq!(
            read(&hex("c70cff00000005fffffffffffffffe")),
            Some(Value::Time { secs: -2, nanos: 5 })
        );
    }

    #[test]
    fn refuses_what_no_host_sends() {
        for (why, h) in [
            ("a nil", "81a161c0"),
            ("0xc1", "c1"),
            ("a key that is not a str", "810101"),
            ("a key given twice", "82a16101a16102"),
            ("a str that is not UTF-8", "a2c328"),
            ("a surrogate", "a3eda080"),
            ("an int past 2^53 − 1", "cf0020000000000000"),
            ("an int past −(2^53 − 1)", "d3ffe0000000000000"),
            (
                "a timestamp of 16 bytes",
                "d8ff00000000000000000000000000000000",
            ),
            ("a timestamp of 1 byte", "d4ff00"),
            (
                "nanoseconds of a whole second",
                "c70cff3b9aca000000000000000000",
            ),
            ("seconds past 2^53 − 1", "c70cff000000000020000000000000"),
            ("cut short", "82a16101"),
            ("bytes after the value", "8000"),
            ("a length past the bytes left", "dfffffffff"),
            ("nothing", ""),
        ] {
            assert_eq!(read(&hex(h)), None, "{why}");
        }
    }

    #[test]
    fn nests_32_levels_and_no_more() {
        let nest = |levels: usize| {
            let mut b = Vec::new();
            for _ in 0..levels - 1 {
                b.extend([0x81, 0xa1, b'x']);
            }
            b.push(0x80);
            b
        };
        assert!(read(&nest(32)).is_some());
        assert!(read(&nest(33)).is_none());
    }
}
