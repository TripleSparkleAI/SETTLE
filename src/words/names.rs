//! Finding a thing by its name, with the error both faces share: a near miss gets `did you mean`.

use crate::engine::model::Model;
use crate::words::lex::{err, suggest, SettleError};

/// The index of the thing called `name`, or the message naming it as unknown (no line: the caller adds one).
pub fn find(m: &Model, name: &str) -> Result<usize, String> {
    match m.idx.get(name) {
        Some(&i) => Ok(i),
        None => Err(match suggest(name, m.names.iter().map(String::as_str)) {
            Some(s) => format!("unknown thing :{}; did you mean :{}? (or declare it with: thing :{})", name, s, name),
            None => format!("unknown thing :{} (declare it with: thing :{})", name, name),
        }),
    }
}

impl Model {
    /// The index of the thing called `name`, or an error on line `ln` naming it (with `did you mean`).
    pub fn need(&self, name: &str, ln: usize) -> Result<usize, SettleError> {
        find(self, name).or_else(|e| err(ln, e))
    }
}
