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
}
