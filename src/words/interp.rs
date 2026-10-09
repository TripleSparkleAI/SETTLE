//! WORDS: blocks (`model :x do ... end`, `run :x do ... end`) and dispatch of each statement line to the statement
//! families through the registry (`words::registry`).

use crate::engine::model::{Model, State, RUN_SEED};
use crate::words::lex::{err, lex, suggest, SettleError, Tok};
use crate::words::registry::{registry, Ctx, Ext};
use std::collections::HashMap;
use std::path::PathBuf;

pub struct Interp {
    pub models: HashMap<String, Model>,
    pub exts: Vec<Box<dyn Ext>>,
    pub base_dir: PathBuf,
}

impl Default for Interp {
    fn default() -> Self {
        Interp { models: HashMap::new(), exts: registry(), base_dir: PathBuf::from(".") }
    }
}

/// The verb of a statement line: `settle` in `settle 100`, `read` in `s.read ...`.
fn verb_of(t: &[Tok]) -> Option<&str> {
    match t {
        [Tok::Ident(_), Tok::Dot, Tok::Ident(v), ..] => Some(v),
        [Tok::Ident(v), ..] => Some(v),
        _ => None,
    }
}

impl Interp {
    /// An interpreter whose relative paths resolve against `dir` (the folder of the program it will run).
    pub fn in_dir(dir: impl Into<PathBuf>) -> Self {
        Interp { base_dir: dir.into(), ..Default::default() }
    }

    /// Every verb the families list for `place` ("model" or "run"), from their `statements()` help lines.
    fn verbs(&self, place: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for e in &self.exts {
            for s in e.statements() {
                let Some((places, body)) = s.split_once(": ") else { continue };
                if !places.split('/').any(|p| p == place) {
                    continue;
                }
                for form in body.split("   /   ") {
                    let first = form.split_whitespace().next().unwrap_or("");
                    let word = first.rsplit('.').next().unwrap_or(first);
                    let word: String = word.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
                    if !word.is_empty() && !out.contains(&word) {
                        out.push(word);
                    }
                }
            }
        }
        out
    }

    pub fn exec(&mut self, src: &str) -> Result<Vec<String>, SettleError> {
        let mut out = Vec::new();
        let mut block: Option<(bool, String, usize)> = None; // (is_model, name, opening line)
        let mut state: Option<State> = None;
        for (n0, raw) in src.lines().enumerate() {
            let ln = n0 + 1;
            let t = lex(raw, ln)?;
            if t.is_empty() {
                continue;
            }
            if let [Tok::Ident(k), Tok::Sym(name), Tok::Ident(d)] = t.as_slice() {
                if (k == "model" || k == "run") && d == "do" {
                    if block.is_some() {
                        return err(ln, "blocks cannot nest; close the previous one with `end`");
                    }
                    if k == "model" {
                        self.models.entry(name.clone()).or_default();
                    } else {
                        if !self.models.contains_key(name) {
                            return err(ln, format!("no model :{} to run", name));
                        }
                        state = Some(State::new(RUN_SEED));
                    }
                    block = Some((k == "model", name.clone(), ln));
                    continue;
                }
            }
            if t == [Tok::Ident("end".into())] {
                if block.is_none() {
                    return err(ln, "`end` without an open block");
                }
                block = None;
                state = None;
                continue;
            }
            let (is_model, name, _) = match &block {
                Some(b) => b.clone(),
                None => return err(ln, "statements live inside `model :name do ... end` or `run :name do ... end`"),
            };
            let m = self.models.get_mut(&name).unwrap();
            let mut ctx = Ctx { base_dir: self.base_dir.clone(), out: &mut out };
            let mut claimed = None;
            for e in &self.exts {
                claimed = if is_model {
                    e.model_stmt(m, &t, ln, &mut ctx)
                } else {
                    e.run_stmt(m, state.as_mut().unwrap(), &t, ln, &mut ctx)
                };
                if claimed.is_some() {
                    break;
                }
            }
            match claimed {
                Some(r) => r?,
                None => {
                    let place = if is_model { "model" } else { "run" };
                    let other = if is_model { "run" } else { "model" };
                    let known: Vec<&str> = self
                        .exts
                        .iter()
                        .flat_map(|e| e.statements().iter().copied())
                        .filter_map(|s| s.strip_prefix(&format!("{}: ", place)))
                        .collect();
                    let hint = match verb_of(&t) {
                        Some(v) if self.verbs(other).iter().any(|w| w == v) && !self.verbs(place).iter().any(|w| w == v) => {
                            format!(" `{}` is a {} statement; put it inside `{} :name do ... end`.", v, other, other)
                        }
                        Some(v) => {
                            let verbs = self.verbs(place);
                            match suggest(v, verbs.iter().map(String::as_str)) {
                                Some(s) => format!(" Did you mean `{}`?", s),
                                None => String::new(),
                            }
                        }
                        None => String::new(),
                    };
                    return err(ln, format!("no statement family knows this line inside a {}.{} Known:\n    {}", place, hint, known.join("\n    ")));
                }
            }
        }
        if let Some((_, name, opened)) = block {
            return err(opened, format!("block :{} is never closed with `end`", name));
        }
        Ok(out)
    }
}
