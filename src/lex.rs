//! Tokens, errors and small argument helpers shared by every SETTLE module.

use std::fmt;

#[derive(Debug)]
pub struct SettleError(pub String);

impl fmt::Display for SettleError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub fn err<T>(ln: usize, msg: impl Into<String>) -> Result<T, SettleError> {
    Err(SettleError(format!("line {}: {}", ln, msg.into())))
}

/// One token. `Sym` is `:name`, `Label` is `name:`, `Str` is a double-quoted string (paths).
#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Sym(String),
    Label(String),
    Ident(String),
    Num(f64),
    Str(String),
    Comma,
    Dot,
}

pub fn lex(line: &str, ln: usize) -> Result<Vec<Tok>, SettleError> {
    let c: Vec<char> = line.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    let word = |c: &[char], mut j: usize| {
        let s = j;
        // A hyphen joins two words into one name when a letter follows it: `read-address:`.
        while j < c.len()
            && (c[j].is_alphanumeric() || c[j] == '_' || (c[j] == '-' && j > s && j + 1 < c.len() && c[j + 1].is_alphabetic()))
        {
            j += 1;
        }
        (c[s..j].iter().collect::<String>(), j)
    };
    while i < c.len() {
        let ch = c[i];
        if ch == '#' {
            break;
        } else if ch.is_whitespace() {
            i += 1;
        } else if ch == '"' {
            let s = i + 1;
            let mut j = s;
            while j < c.len() && c[j] != '"' {
                j += 1;
            }
            if j >= c.len() {
                return err(ln, "a string is missing its closing quote");
            }
            out.push(Tok::Str(c[s..j].iter().collect()));
            i = j + 1;
        } else if ch == ',' {
            out.push(Tok::Comma);
            i += 1;
        } else if ch == '.' && !(i + 1 < c.len() && c[i + 1].is_ascii_digit()) {
            out.push(Tok::Dot);
            i += 1;
        } else if ch == ':' {
            let (w, j) = word(&c, i + 1);
            if w.is_empty() {
                return err(ln, "a ':' must start a symbol like :rain");
            }
            out.push(Tok::Sym(w));
            i = j;
        } else if ch.is_ascii_digit() || ch == '-' || ch == '.' {
            let s = i;
            i += 1;
            while i < c.len() && (c[i].is_ascii_digit() || c[i] == '_' || c[i] == '.' || c[i] == 'e') {
                i += 1;
            }
            let txt: String = c[s..i].iter().filter(|&&x| x != '_').collect();
            match txt.parse::<f64>() {
                Ok(v) => out.push(Tok::Num(v)),
                Err(_) => return err(ln, format!("'{}' is not a number", txt)),
            }
        } else if ch.is_alphabetic() || ch == '_' {
            let (w, j) = word(&c, i);
            if j < c.len() && c[j] == ':' && !(j + 1 < c.len() && c[j + 1] == ':') {
                out.push(Tok::Label(w));
                i = j + 1;
            } else {
                out.push(Tok::Ident(w));
                i = j;
            }
        } else {
            return err(ln, format!("unexpected '{}'", ch));
        }
    }
    Ok(out)
}

/// `key: value` pairs after the positional part of a statement.
pub fn kwargs(toks: &[Tok], ln: usize) -> Result<Vec<(String, Tok)>, SettleError> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < toks.len() {
        match &toks[i] {
            Tok::Comma => i += 1,
            Tok::Label(k) if i + 1 < toks.len() => {
                out.push((k.clone(), toks[i + 1].clone()));
                i += 2;
            }
            t => return err(ln, format!("expected `key: value`, found {:?}", t)),
        }
    }
    Ok(out)
}

/// Look up a keyword argument by name.
pub fn kw<'a>(kv: &'a [(String, Tok)], key: &str) -> Option<&'a Tok> {
    kv.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

/// Keywords retired for Kanerva's own terms (experiments/thermosim/kanerva/KANERVA_TERMS.md): the old word,
/// the new word, and Kanerva's phrase for it.
pub const RETIRED: &[(&str, &str, &str)] = &[
    ("cue", "read-address", "Kanerva's retrieval address"),
    ("damage", "address-noise", "Kanerva's noise in the address"),
    ("locations", "hard-locations", "Kanerva's hard locations"),
    ("radius", "activation-radius", "Kanerva's activation radius"),
    ("fire", "activation-probability", "Kanerva's probability of activation"),
    ("iterations", "iterated-reads", "Kanerva's iterated reading"),
    ("size", "word-size", "Kanerva's word size"),
    ("tolerate", "tolerate-noise", "the address-noise to tolerate"),
];

/// Refuse any keyword not in `allowed`, naming the statement. A retired keyword whose new word this
/// statement takes is refused with the new word, so `cue:` says to write `read-address:`.
pub fn only(kv: &[(String, Tok)], allowed: &[&str], what: &str, ln: usize) -> Result<(), SettleError> {
    for (k, _) in kv {
        if !allowed.contains(&k.as_str()) {
            if let Some((_, new, why)) = RETIRED.iter().find(|(old, new, _)| old == k && allowed.contains(new)) {
                return err(ln, format!("`{}:` is now `{}:` ({})", k, new, why));
            }
            return err(ln, format!("{} does not take `{}:`", what, k));
        }
    }
    Ok(())
}

pub fn num(t: &Tok, ln: usize) -> Result<f64, SettleError> {
    match t {
        Tok::Num(v) => Ok(*v),
        _ => err(ln, "a number was expected"),
    }
}

pub fn text(t: &Tok, ln: usize) -> Result<String, SettleError> {
    match t {
        Tok::Str(s) => Ok(s.clone()),
        _ => err(ln, "a \"quoted\" string was expected"),
    }
}

pub fn yes_no(t: &Tok, ln: usize) -> Result<f64, SettleError> {
    match t {
        Tok::Sym(s) if s == "yes" => Ok(1.0),
        Tok::Sym(s) if s == "no" => Ok(-1.0),
        _ => err(ln, "expected :yes or :no"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hyphen_between_letters_joins_one_label() {
        let t = lex("s.read read-address: :cat, address-noise: 0.2", 1).unwrap();
        assert!(t.contains(&Tok::Label("read-address".into())), "{:?}", t);
        assert!(t.contains(&Tok::Label("address-noise".into())), "{:?}", t);
        let t = lex("x :hard-locations", 1).unwrap();
        assert!(t.contains(&Tok::Sym("hard-locations".into())), "{:?}", t);
    }

    #[test]
    fn a_minus_before_a_number_stays_a_number() {
        assert_eq!(lex("by: -0.5", 1).unwrap(), vec![Tok::Label("by".into()), Tok::Num(-0.5)]);
        assert_eq!(lex("leans: -2", 1).unwrap(), vec![Tok::Label("leans".into()), Tok::Num(-2.0)]);
        // a trailing hyphen is not part of the word
        assert!(lex("a- 1", 1).is_err() || lex("a- 1", 1).unwrap()[0] == Tok::Ident("a".into()));
    }

    #[test]
    fn a_retired_keyword_names_its_new_word() {
        let kv = kwargs(&lex(concat!("cu", "e: :cat"), 1).unwrap(), 1).unwrap();
        let e = only(&kv, &["read-address", "address-noise"], "read", 3).unwrap_err().0;
        assert_eq!(e, "line 3: `cue:` is now `read-address:` (Kanerva's retrieval address)");
        // `size:` is retired only where `word-size:` is taken: the Hopfield memory keeps `size:`
        let kv = kwargs(&lex("size: 64", 1).unwrap(), 1).unwrap();
        assert!(only(&kv, &["size", "fade"], "memory", 1).is_ok());
        assert!(only(&kv, &["word-size", "hard-locations"], "sdm", 1).unwrap_err().0.contains("`word-size:`"));
        // an unknown keyword still gets the plain refusal
        let kv = kwargs(&lex("colour: 1", 1).unwrap(), 1).unwrap();
        assert_eq!(only(&kv, &["word-size"], "sdm", 1).unwrap_err().0, "line 1: sdm does not take `colour:`");
    }
}
