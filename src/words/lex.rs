//! WORDS: tokens, errors and the small argument helpers every statement family parses with.

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

/// A token as it is written in a program: `:rain`, `by:`, `settle`, `2.5`, `"a.pgm"`, `,`, `.`. Error messages
/// quote tokens in this form, between backticks, so the caret under the line can find them.
impl fmt::Display for Tok {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Tok::Sym(s) => write!(f, ":{}", s),
            Tok::Label(s) => write!(f, "{}:", s),
            Tok::Ident(s) => write!(f, "{}", s),
            Tok::Num(v) => write!(f, "{}", v),
            Tok::Str(s) => write!(f, "\"{}\"", s),
            Tok::Comma => write!(f, ","),
            Tok::Dot => write!(f, "."),
        }
    }
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
            t => return err(ln, format!("expected `key: value`, found `{}`", t)),
        }
    }
    Ok(out)
}

/// Look up a keyword argument by name.
pub fn kw<'a>(kv: &'a [(String, Tok)], key: &str) -> Option<&'a Tok> {
    kv.iter().find(|(k, _)| k == key).map(|(_, v)| v)
}

/// Keywords retired for Kanerva's own terms (SETTLE/kanerva/KANERVA_TERMS.md): the old word,
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
/// statement takes is refused with the new word, so `cue:` says to write `read-address:`. Any other unknown
/// keyword is refused with the nearest keyword the statement takes when one is close (`did you mean`), and
/// otherwise with the list of keywords it takes.
pub fn only(kv: &[(String, Tok)], allowed: &[&str], what: &str, ln: usize) -> Result<(), SettleError> {
    for (k, _) in kv {
        if !allowed.contains(&k.as_str()) {
            if let Some((_, new, why)) = RETIRED.iter().find(|(old, new, _)| old == k && allowed.contains(new)) {
                return err(ln, format!("`{}:` is now `{}:` ({})", k, new, why));
            }
            let hint = match suggest(k, allowed.iter().copied()) {
                Some(s) => format!("; did you mean `{}:`?", s),
                None if allowed.is_empty() => format!("; {} takes no `key: value` arguments", what),
                None => format!("; it takes {}", list_of(allowed.iter().map(|a| format!("`{}:`", a)))),
            };
            return err(ln, format!("{} does not take `{}:`{}", what, k, hint));
        }
    }
    Ok(())
}

/// `a`, `a and b`, `a, b and c`: a list for an error message.
pub fn list_of(items: impl IntoIterator<Item = String>) -> String {
    let v: Vec<String> = items.into_iter().collect();
    match v.len() {
        0 => String::new(),
        1 => v[0].clone(),
        n => format!("{} and {}", v[..n - 1].join(", "), v[n - 1]),
    }
}

/// The edit distance between two words: insertions, deletions, substitutions and swaps of two neighbouring letters
/// cost one each (the optimal string alignment distance), with `-` and `_` counted as the same character, so
/// `warm-fit` and `warm_fit` are 0 apart and `rian` is 1 from `rain`.
pub fn edit_distance(a: &str, b: &str) -> usize {
    let norm = |c: char| if c == '-' { '_' } else { c.to_ascii_lowercase() };
    let a: Vec<char> = a.chars().map(norm).collect();
    let b: Vec<char> = b.chars().map(norm).collect();
    let (n, m) = (a.len(), b.len());
    let mut d = vec![vec![0usize; m + 1]; n + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=n {
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut v = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(d[i - 2][j - 2] + 1);
            }
            d[i][j] = v;
        }
    }
    d[n][m]
}

/// The candidate nearest to `word` when it is close enough to be a likely slip: at most one edit for a word of up
/// to five letters, at most a third of the length for a longer one, and always fewer edits than the shorter word
/// has letters (so `:b` is never "corrected" to `:a`). Ties go to the first candidate. `None` when nothing is close.
pub fn suggest<'a>(word: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let limit = (word.chars().count() / 3).max(1);
    let mut best: Option<(usize, &'a str)> = None;
    for c in candidates {
        if c == word {
            continue;
        }
        let d = edit_distance(word, c);
        let shorter = word.chars().count().min(c.chars().count());
        if d <= limit && d < shorter && best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, c));
        }
    }
    best.map(|(_, c)| c)
}

/// Where in the program an error points: `(line, column, width)`, all counted from 1 in characters. The line is the
/// error's `line N:`. The column is the first thing the message quotes that appears on that line: a `backticked`
/// fragment, then a `:symbol`, read from the message's first line only; failing both, the statement's first
/// character, with the whole statement as the width.
/// `None` when the message names no line inside the program.
pub fn locate(src: &str, msg: &str) -> Option<(usize, usize, usize)> {
    let rest = msg.strip_prefix("line ")?;
    let n_end = rest.find(':')?;
    let ln: usize = rest[..n_end].parse().ok()?;
    let line = src.lines().nth(ln.checked_sub(1)?)?;
    // only the message's first line: later lines (a list of known statements) quote other programs
    let body = rest[n_end + 1..].lines().next().unwrap_or("");
    let code_end = line.find('#').unwrap_or(line.len());
    let code = &line[..code_end];
    let col_of = |byte: usize| code[..byte].chars().count() + 1;
    // a `backticked` fragment of the message, as written on the line
    let mut parts = body.split('`');
    parts.next();
    while let Some(frag) = parts.next() {
        if !frag.is_empty() {
            if let Some(b) = code.find(frag) {
                return Some((ln, col_of(b), frag.chars().count()));
            }
        }
        parts.next();
    }
    // a :symbol of the message, as a whole word on the line
    for word in body.split(|c: char| c.is_whitespace() || c == ',' || c == '(' || c == ')' || c == ';') {
        let sym = word.trim_end_matches(['.', '?', '!']);
        if sym.len() > 1 && sym.starts_with(':') {
            let mut from = 0;
            while let Some(b) = code[from..].find(sym) {
                let at = from + b;
                let after = code[at + sym.len()..].chars().next();
                if !after.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '-') {
                    return Some((ln, col_of(at), sym.chars().count()));
                }
                from = at + sym.len();
            }
        }
    }
    let start = code.len() - code.trim_start().len();
    let width = code.trim().chars().count().max(1);
    Some((ln, col_of(start), width))
}

/// A whole number from `lo` to `hi` (`hi` may be `f64::INFINITY`), or the family convention's refusal:
/// `<what> takes a whole number from <lo> to <hi>` (or `of <lo> or more`). A fraction or a number out of range is
/// refused, never truncated.
pub fn whole(v: f64, lo: f64, hi: f64, what: &str, ln: usize) -> Result<usize, SettleError> {
    if v < lo || v > hi || v.fract() != 0.0 || v.is_nan() {
        return if hi.is_infinite() {
            err(ln, format!("{} takes a whole number of {} or more; got {}", what, lo, v))
        } else {
            err(ln, format!("{} takes a whole number from {} to {}", what, lo, hi))
        };
    }
    Ok(v as usize)
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
        // an unknown keyword far from every allowed one gets the list of what the statement takes
        let kv = kwargs(&lex("colour: 1", 1).unwrap(), 1).unwrap();
        assert_eq!(only(&kv, &["word-size"], "sdm", 1).unwrap_err().0, "line 1: sdm does not take `colour:`; it takes `word-size:`");
        assert_eq!(
            only(&kv, &["seed", "temperature"], "settle", 1).unwrap_err().0,
            "line 1: settle does not take `colour:`; it takes `seed:` and `temperature:`"
        );
        assert_eq!(only(&kv, &[], "show", 1).unwrap_err().0, "line 1: show does not take `colour:`; show takes no `key: value` arguments");
    }

    #[test]
    fn a_near_miss_keyword_gets_a_did_you_mean() {
        let kv = kwargs(&lex("sed: 1", 1).unwrap(), 1).unwrap();
        assert_eq!(only(&kv, &["seed", "temperature"], "settle", 4).unwrap_err().0, "line 4: settle does not take `sed:`; did you mean `seed:`?");
        // a hyphen and an underscore count as the same letter, so the right spelling is named
        let kv = kwargs(&lex("write-samples: 16", 1).unwrap(), 1).unwrap();
        assert!(only(&kv, &["word-size", "write_samples"], "softsdm", 1).unwrap_err().0.ends_with("did you mean `write_samples:`?"));
    }

    #[test]
    fn a_token_in_an_error_is_quoted_as_written() {
        let e = kwargs(&lex("settle 10, 5", 1).unwrap()[1..], 4).unwrap_err().0;
        assert_eq!(e, "line 4: expected `key: value`, found `10`");
        let shown: Vec<String> = lex(r#":rain, by: 2.5 x . "a.pgm""#, 1).unwrap().iter().map(|t| t.to_string()).collect();
        assert_eq!(shown, [":rain", ",", "by:", "2.5", "x", ".", "\"a.pgm\""]);
    }

    #[test]
    fn suggest_names_only_close_candidates() {
        assert_eq!(suggest("temprature", ["seed", "temperature"]), Some("temperature"));
        assert_eq!(suggest("rain", ["rian", "sprinkler"]), Some("rian"));
        // one letter is never corrected into another one-letter word
        assert_eq!(suggest("b", ["a"]), None);
        assert_eq!(suggest("colour", ["word-size", "seed"]), None);
        assert_eq!(edit_distance("warm-fit", "warm_fit"), 0);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
    }

    #[test]
    fn locate_points_at_what_the_message_quotes() {
        let src = "model :m do\n  thing :rain\nend\nrun :m do\n  settle 100, sed: 1   # a slip\nend";
        assert_eq!(locate(src, "line 5: settle does not take `sed:`; did you mean `seed:`?"), Some((5, 15, 4)));
        let src2 = "model :m do\n  thing :a\n  a.pulls :zz, by: 1\nend";
        assert_eq!(locate(src2, "line 3: unknown thing :zz (declare it with: thing :zz)"), Some((3, 11, 3)));
        // nothing quoted that is on the line: the statement itself
        assert_eq!(locate(src2, "line 3: something went wrong"), Some((3, 3, 18)));
        assert_eq!(locate(src2, "line 9: past the end"), None);
        // a later line of the message is not searched
        assert_eq!(locate(src2, "line 3: no family knows this line. Known:\n    a.pulls :zz"), Some((3, 3, 18)));
        assert_eq!(locate(src2, "cannot read x"), None);
    }
}
