//! THE BUILDER FACE: the core family written in Rust, Rails-style, over the same word list as the file face.
//!
//! ```
//! use settle::engine::model::Model;
//! let weather = Model::build()
//!     .thing("rain").leans("no", 1.0)
//!     .thing("sprinkler").leans("no", 0.5)
//!     .thing("wet_grass")
//!     .pushes("rain", "sprinkler", 0.5)
//!     .pulls("rain", "wet_grass", 1.5)
//!     .pulls("sprinkler", "wet_grass", 1.0);
//! let lines = weather
//!     .run()
//!     .hold("wet_grass", "yes")
//!     .settle(20_000).temperature(1.0).seed(1)
//!     .show()
//!     .ask("rain").and("sprinkler")
//!     .lines()
//!     .unwrap();
//! assert_eq!(lines[0], "settled: 20000 samples of 3 things at temperature 1");
//! assert_eq!(lines[4], "ask :rain, and: :sprinkler: yes 31.8% of 20000 samples");
//! ```
//!
//! Those lines are exactly what `docs/examples/core-weather.settle` prints, because both faces build the same
//! statements (`words::core::ModelStmt` and `RunStmt`) and run them through the one executor. The method names
//! are the words of `words::vocab::CORE`; the parameter words, `by`, `yes`/`no` and `gibbs`/`metro`, are the
//! arguments of `leans`, `pulls`, `pushes`, `hold` and `update`.
//!
//! A misuse (`temperature` with no `settle` before it, `leans("maybe", 1.0)`) is recorded where it happens and
//! returned by `model()` or `lines()`, so a chain never panics.

use crate::engine::answers::Join;
use crate::engine::model::{Model, State, RUN_SEED};
use crate::words::core::{apply_model, apply_run, ModelStmt, RunStmt};
use crate::words::lex::SettleError;
use crate::words::vocab as w;

/// One name or several, for `thing`: `"rain"`, `["a", "b"]`, or a `Vec<String>`.
pub trait Names {
    fn names(self) -> Vec<String>;
}

impl Names for &str {
    fn names(self) -> Vec<String> {
        vec![self.to_string()]
    }
}

impl Names for String {
    fn names(self) -> Vec<String> {
        vec![self]
    }
}

impl<const N: usize> Names for [&str; N] {
    fn names(self) -> Vec<String> {
        self.iter().map(|s| s.to_string()).collect()
    }
}

impl Names for &[&str] {
    fn names(self) -> Vec<String> {
        self.iter().map(|s| s.to_string()).collect()
    }
}

impl Names for Vec<String> {
    fn names(self) -> Vec<String> {
        self
    }
}

fn yes_no(word: &str) -> Result<f64, String> {
    match word {
        w::YES => Ok(1.0),
        w::NO => Ok(-1.0),
        other => Err(format!("expected \"yes\" or \"no\", got {:?}", other)),
    }
}

/// A model under construction: the statements so far, and the first misuse if there was one.
#[derive(Clone, Debug, Default)]
pub struct ModelBuilder {
    stmts: Vec<ModelStmt>,
    misuse: Option<String>,
}

impl Model {
    /// Start a model with the builder face (see `words::builder`).
    pub fn build() -> ModelBuilder {
        ModelBuilder::default()
    }
}

impl ModelBuilder {
    fn fail(mut self, msg: String) -> Self {
        self.misuse.get_or_insert(msg);
        self
    }

    /// `thing :a` / `thing :a, :b`: declare one or more things.
    pub fn thing(mut self, names: impl Names) -> Self {
        self.stmts.push(ModelStmt::Thing { names: names.names(), lean: None });
        self
    }

    /// `leans: :yes, by: 1`: the lean of the things the last `thing` declared.
    pub fn leans(mut self, yes_or_no: &str, by: f64) -> Self {
        let l = match yes_no(yes_or_no) {
            Ok(l) => l,
            Err(e) => return self.fail(format!("{}: {}", w::LEANS, e)),
        };
        match self.stmts.last_mut() {
            Some(ModelStmt::Thing { lean, .. }) if lean.is_none() => {
                *lean = Some((l, by));
                self
            }
            Some(ModelStmt::Thing { .. }) => self.fail("`leans` given twice for one `thing`".into()),
            _ => self.fail("`leans` follows a `thing`".into()),
        }
    }

    /// `a.pulls :b, by: 2`: pull two things towards agreeing.
    pub fn pulls(mut self, a: &str, b: &str, by: f64) -> Self {
        self.stmts.push(ModelStmt::Couple { a: a.into(), b: b.into(), pull: true, by });
        self
    }

    /// `a.pushes :b, by: 2`: push two things towards disagreeing.
    pub fn pushes(mut self, a: &str, b: &str, by: f64) -> Self {
        self.stmts.push(ModelStmt::Couple { a: a.into(), b: b.into(), pull: false, by });
        self
    }

    /// The statements built so far, as the file face would parse them.
    pub fn statements(&self) -> &[ModelStmt] {
        &self.stmts
    }

    /// The model, or the first misuse or error.
    pub fn model(&self) -> Result<Model, SettleError> {
        if let Some(e) = &self.misuse {
            return Err(SettleError(format!("builder: {}", e)));
        }
        let mut m = Model::default();
        for (k, s) in self.stmts.iter().enumerate() {
            apply_model(&mut m, s).map_err(|e| SettleError(format!("model statement {}: {}", k + 1, e)))?;
        }
        Ok(m)
    }

    /// Start a run of this model, as `run :name do ... end` does.
    pub fn run(self) -> RunBuilder {
        RunBuilder { model: self, stmts: Vec::new(), misuse: None }
    }
}

/// A run under construction.
#[derive(Clone, Debug)]
pub struct RunBuilder {
    model: ModelBuilder,
    stmts: Vec<RunStmt>,
    misuse: Option<String>,
}

impl RunBuilder {
    fn fail(mut self, msg: String) -> Self {
        self.misuse.get_or_insert(msg);
        self
    }

    fn push(mut self, s: RunStmt) -> Self {
        self.stmts.push(s);
        self
    }

    /// `hold :a, :yes`.
    pub fn hold(self, thing: &str, yes_or_no: &str) -> Self {
        match yes_no(yes_or_no) {
            Ok(value) => self.push(RunStmt::Hold { thing: thing.into(), value }),
            Err(e) => self.fail(format!("{}: {}", w::HOLD, e)),
        }
    }

    /// `settle 10_000`; follow with `temperature` and `seed` to set them.
    pub fn settle(self, sweeps: usize) -> Self {
        self.push(RunStmt::Settle { sweeps, temperature: None, seed: None, update: None })
    }

    /// `anneal 4_000`; follow with `temperature` and `seed` to set them.
    pub fn anneal(self, sweeps: usize) -> Self {
        self.push(RunStmt::Anneal { sweeps, temperature: None, seed: None, update: None })
    }

    /// `temperature: 1` of the last `settle` or `anneal`.
    pub fn temperature(mut self, t: f64) -> Self {
        match self.stmts.last_mut() {
            Some(RunStmt::Settle { temperature, .. }) | Some(RunStmt::Anneal { temperature, .. }) => {
                *temperature = Some(t);
                self
            }
            _ => self.fail("`temperature` follows a `settle` or an `anneal`".into()),
        }
    }

    /// `seed: 1` of the last `settle` or `anneal`.
    pub fn seed(mut self, s: u64) -> Self {
        match self.stmts.last_mut() {
            Some(RunStmt::Settle { seed, .. }) | Some(RunStmt::Anneal { seed, .. }) => {
                *seed = Some(s);
                self
            }
            _ => self.fail("`seed` follows a `settle` or an `anneal`".into()),
        }
    }

    /// `update: :gibbs` or `update: :metro` of the last `settle` or `anneal`; the rule then holds for the rest of
    /// the run, as a `temperature:` does.
    pub fn update(mut self, rule: &str) -> Self {
        let Some(u) = crate::words::core::update_of(rule) else {
            return self.fail(format!("{}: expected \"gibbs\" or \"metro\", got {:?}", w::UPDATE, rule));
        };
        match self.stmts.last_mut() {
            Some(RunStmt::Settle { update, .. }) | Some(RunStmt::Anneal { update, .. }) => {
                *update = Some(u);
                self
            }
            _ => self.fail("`update` follows a `settle` or an `anneal`".into()),
        }
    }

    /// `show`.
    pub fn show(self) -> Self {
        self.push(RunStmt::Show)
    }

    /// `best`.
    pub fn best(self) -> Self {
        self.push(RunStmt::Best)
    }

    /// `ask :a`; follow with `and`, `or`, `and_not` and `or_not` to join more things.
    pub fn ask(self, thing: &str) -> Self {
        self.push(RunStmt::Ask { first: thing.into(), terms: Vec::new() })
    }

    fn join(mut self, j: Join, thing: &str) -> Self {
        match self.stmts.last_mut() {
            Some(RunStmt::Ask { terms, .. }) => {
                terms.push((j, thing.into()));
                self
            }
            _ => self.fail(format!("`{}` follows an `ask`", crate::words::core::join_word(j))),
        }
    }

    /// `and: :b` of the last `ask`.
    pub fn and(self, thing: &str) -> Self {
        self.join(Join::And, thing)
    }

    /// `or: :b` of the last `ask`.
    pub fn or(self, thing: &str) -> Self {
        self.join(Join::Or, thing)
    }

    /// `and_not: :b` of the last `ask`.
    pub fn and_not(self, thing: &str) -> Self {
        self.join(Join::AndNot, thing)
    }

    /// `or_not: :b` of the last `ask`.
    pub fn or_not(self, thing: &str) -> Self {
        self.join(Join::OrNot, thing)
    }

    /// The run statements built so far.
    pub fn statements(&self) -> &[RunStmt] {
        &self.stmts
    }

    /// Build the model, run every statement in order, and return the printed lines.
    pub fn lines(&self) -> Result<Vec<String>, SettleError> {
        if let Some(e) = &self.misuse {
            return Err(SettleError(format!("builder: {}", e)));
        }
        let m = self.model.model()?;
        let mut st = State::new(RUN_SEED);
        let mut out = Vec::new();
        for (k, s) in self.stmts.iter().enumerate() {
            apply_run(&m, &mut st, s, &mut out).map_err(|e| SettleError(format!("run statement {}: {}", k + 1, e)))?;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use crate::engine::model::Model;

    #[test]
    fn a_misuse_is_returned_not_panicked() {
        let e = Model::build().leans("yes", 1.0).model().unwrap_err().0;
        assert_eq!(e, "builder: `leans` follows a `thing`");
        let e = Model::build().thing("a").leans("maybe", 1.0).model().unwrap_err().0;
        assert_eq!(e, "builder: leans: expected \"yes\" or \"no\", got \"maybe\"");
        let e = Model::build().thing("a").run().temperature(1.0).lines().unwrap_err().0;
        assert_eq!(e, "builder: `temperature` follows a `settle` or an `anneal`");
        let e = Model::build().thing("a").run().and("a").lines().unwrap_err().0;
        assert_eq!(e, "builder: `and` follows an `ask`");
    }

    #[test]
    fn engine_errors_name_the_statement_and_match_the_file_face_words() {
        let e = Model::build().thing("rain").pulls("rain", "rian", 1.0).model().unwrap_err().0;
        assert_eq!(e, "model statement 2: unknown thing :rian; did you mean :rain? (or declare it with: thing :rian)");
        let e = Model::build().thing("a").run().show().lines().unwrap_err().0;
        assert_eq!(e, "run statement 1: show needs a settle first");
        let e = Model::build().thing("a").run().settle(10).temperature(0.0).lines().unwrap_err().0;
        assert_eq!(e, "run statement 1: temperature must be above zero");
        let e = Model::build().thing("yes").model().unwrap_err().0;
        assert_eq!(e, "model statement 1: :yes cannot be a thing name");
    }
}
