//! JSON in and out, with no dependencies: a writer for numbers and strings, a small reader (enough for the
//! `export`/`import` files and the command's own answers), and the `--json` answer of the `settle` command.

use std::fmt::Write as _;

/// A finite number in Rust's shortest form that parses back to the same f64; refuses NaN and infinities.
pub fn number(x: f64) -> Result<String, String> {
    if !x.is_finite() {
        return Err(format!("{} cannot be written as JSON", x));
    }
    Ok(format!("{:?}", x))
}

/// A string as a JSON string literal, with quotes, backslashes and control characters escaped.
pub fn string(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                let _ = write!(o, "\\u{:04x}", c as u32);
            }
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(kv) => kv.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
}

struct P<'a> {
    b: &'a [u8],
    i: usize,
}

impl P<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && self.b[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }
    fn fail<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("JSON: {} at byte {}", what, self.i))
    }
    fn lit(&mut self, s: &str, v: Json) -> Result<Json, String> {
        if self.b[self.i..].starts_with(s.as_bytes()) {
            self.i += s.len();
            Ok(v)
        } else {
            self.fail("unknown word")
        }
    }
    fn value(&mut self) -> Result<Json, String> {
        self.ws();
        match self.b.get(self.i) {
            None => self.fail("unexpected end"),
            Some(b'{') => {
                self.i += 1;
                let mut kv = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Ok(Json::Obj(kv));
                }
                loop {
                    self.ws();
                    let k = match self.value()? {
                        Json::Str(s) => s,
                        _ => return self.fail("object key must be a string"),
                    };
                    self.ws();
                    if self.b.get(self.i) != Some(&b':') {
                        return self.fail("expected ':'");
                    }
                    self.i += 1;
                    let v = self.value()?;
                    kv.push((k, v));
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Json::Obj(kv));
                        }
                        _ => return self.fail("expected ',' or '}'"),
                    }
                }
            }
            Some(b'[') => {
                self.i += 1;
                let mut v = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Ok(Json::Arr(v));
                }
                loop {
                    v.push(self.value()?);
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Json::Arr(v));
                        }
                        _ => return self.fail("expected ',' or ']'"),
                    }
                }
            }
            Some(b'"') => {
                self.i += 1;
                let mut s = String::new();
                loop {
                    match self.b.get(self.i) {
                        None => return self.fail("unclosed string"),
                        Some(b'"') => {
                            self.i += 1;
                            return Ok(Json::Str(s));
                        }
                        Some(b'\\') => {
                            let c = self.b.get(self.i + 1).copied();
                            self.i += 2;
                            match c {
                                Some(b'"') => s.push('"'),
                                Some(b'\\') => s.push('\\'),
                                Some(b'/') => s.push('/'),
                                Some(b'n') => s.push('\n'),
                                Some(b't') => s.push('\t'),
                                Some(b'r') => s.push('\r'),
                                Some(b'b') => s.push('\u{8}'),
                                Some(b'f') => s.push('\u{c}'),
                                Some(b'u') => {
                                    let hex = std::str::from_utf8(self.b.get(self.i..self.i + 4).unwrap_or(&[])).unwrap_or("");
                                    let cp = u32::from_str_radix(hex, 16).map_err(|_| "JSON: bad \\u escape".to_string())?;
                                    s.push(char::from_u32(cp).unwrap_or('\u{fffd}'));
                                    self.i += 4;
                                }
                                _ => return self.fail("bad escape"),
                            }
                        }
                        Some(_) => {
                            // copy one UTF-8 character
                            let rest = std::str::from_utf8(&self.b[self.i..]).map_err(|_| "JSON: not UTF-8".to_string())?;
                            let c = rest.chars().next().unwrap();
                            s.push(c);
                            self.i += c.len_utf8();
                        }
                    }
                }
            }
            Some(b't') => self.lit("true", Json::Bool(true)),
            Some(b'f') => self.lit("false", Json::Bool(false)),
            Some(b'n') => self.lit("null", Json::Null),
            Some(_) => {
                let s = self.i;
                while self.i < self.b.len() && matches!(self.b[self.i], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
                    self.i += 1;
                }
                let t = std::str::from_utf8(&self.b[s..self.i]).unwrap_or("");
                t.parse::<f64>().map(Json::Num).or_else(|_| self.fail(&format!("'{}' is not a number", t)))
            }
        }
    }
}

pub fn parse_json(s: &str) -> Result<Json, String> {
    let mut p = P { b: s.as_bytes(), i: 0 };
    let v = p.value()?;
    p.ws();
    if p.i != p.b.len() {
        return p.fail("trailing text");
    }
    Ok(v)
}

/// The `settle --json` answer for a program that ran: `{"settle": version, "ok": true, "lines": [...]}`.
pub fn answer_ok(version: &str, lines: &[String]) -> String {
    let ls: Vec<String> = lines.iter().map(|l| string(l)).collect();
    format!("{{\"settle\": {}, \"ok\": true, \"lines\": [{}]}}", string(version), ls.join(", "))
}

/// The `settle --json` answer for a program that failed: the message, and where it points when it names a line
/// of the program (`line`, `column` and `width` count from 1, in characters, as the caret does).
pub fn answer_error(version: &str, message: &str, at: Option<(usize, usize, usize)>) -> String {
    let place = match at {
        Some((ln, col, width)) => format!(", \"line\": {}, \"column\": {}, \"width\": {}", ln, col, width),
        None => String::new(),
    };
    format!("{{\"settle\": {}, \"ok\": false, \"error\": {{\"message\": {}{}}}}}", string(version), string(message), place)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_reads_back_as_json() {
        let ok = answer_ok("0.1.0", &["settled: 3 samples".into(), "a \"quoted\" \\ line\twith a tab".into()]);
        let j = parse_json(&ok).unwrap();
        assert_eq!(j.get("ok"), Some(&Json::Bool(true)));
        assert_eq!(j.get("lines"), Some(&Json::Arr(vec![Json::Str("settled: 3 samples".into()), Json::Str("a \"quoted\" \\ line\twith a tab".into())])));
        let e = answer_error("0.1.0", "line 2: unknown thing :zz", Some((2, 11, 3)));
        let j = parse_json(&e).unwrap();
        assert_eq!(j.get("ok"), Some(&Json::Bool(false)));
        let err = j.get("error").unwrap();
        assert_eq!(err.get("line"), Some(&Json::Num(2.0)));
        assert_eq!(err.get("column"), Some(&Json::Num(11.0)));
        assert_eq!(err.get("message"), Some(&Json::Str("line 2: unknown thing :zz".into())));
        assert!(parse_json(&answer_error("0.1.0", "cannot read x", None)).unwrap().get("error").unwrap().get("line").is_none());
    }

    #[test]
    fn numbers_round_trip_and_non_finite_is_refused() {
        for x in [0.0, 1.0, -0.5, 0.1 + 0.2, 1e-300, 123456789.125] {
            assert_eq!(parse_json(&number(x).unwrap()).unwrap(), Json::Num(x));
        }
        assert!(number(f64::NAN).is_err() && number(f64::INFINITY).is_err());
    }
}
