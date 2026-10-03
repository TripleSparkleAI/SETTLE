//! Blocks (`model :x do ... end`, `run :x do ... end`) and dispatch of each statement to the statement families.

use crate::ext::{registry, Ctx, Ext};
use crate::lex::{err, lex, SettleError, Tok};
use crate::model::{Model, State};
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

impl Interp {
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
                        state = Some(State::new(0x5eed));
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
                    let known: Vec<&str> = self
                        .exts
                        .iter()
                        .flat_map(|e| e.statements().iter().copied())
                        .filter_map(|s| s.strip_prefix(&format!("{}: ", place)))
                        .collect();
                    return err(ln, format!("no statement family knows this line inside a {}. Known:\n    {}", place, known.join("\n    ")));
                }
            }
        }
        if let Some((_, name, opened)) = block {
            return err(opened, format!("block :{} is never closed with `end`", name));
        }
        Ok(out)
    }
}
