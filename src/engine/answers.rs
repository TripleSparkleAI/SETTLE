//! Answers by counting: how often a combination of things was yes across a settle's kept samples.

/// How one more thing joins a question.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Join {
    /// both yes
    And,
    /// at least one yes
    Or,
    /// the question so far yes and this thing no
    AndNot,
    /// the question so far yes, or this thing no
    OrNot,
}

impl Join {
    /// Combine the answer so far with one more thing's value.
    pub fn apply(self, so_far: bool, thing: bool) -> bool {
        match self {
            Join::And => so_far && thing,
            Join::Or => so_far || thing,
            Join::AndNot => so_far && !thing,
            Join::OrNot => so_far || !thing,
        }
    }
}

/// A question about a run: a first thing, then each further thing joined left to right.
#[derive(Clone, Debug, PartialEq)]
pub struct Question {
    pub first: usize,
    pub terms: Vec<(Join, usize)>,
}

/// How many of `samples` answer the question yes, and of how many.
///
/// ```
/// use settle::engine::answers::{count_yes, Join, Question};
/// let samples = vec![vec![1.0, 1.0], vec![1.0, -1.0], vec![-1.0, 1.0], vec![-1.0, -1.0]];
/// assert_eq!(count_yes(&samples, &Question { first: 0, terms: vec![] }), (2, 4));
/// assert_eq!(count_yes(&samples, &Question { first: 0, terms: vec![(Join::And, 1)] }), (1, 4));
/// assert_eq!(count_yes(&samples, &Question { first: 0, terms: vec![(Join::Or, 1)] }), (3, 4));
/// assert_eq!(count_yes(&samples, &Question { first: 0, terms: vec![(Join::AndNot, 1)] }), (1, 4));
/// assert_eq!(count_yes(&samples, &Question { first: 0, terms: vec![(Join::OrNot, 1)] }), (3, 4));
/// ```
pub fn count_yes(samples: &[Vec<f64>], q: &Question) -> (usize, usize) {
    let yes = samples
        .iter()
        .filter(|s| q.terms.iter().fold(s[q.first] > 0.0, |x, &(j, k)| j.apply(x, s[k] > 0.0)))
        .count();
    (yes, samples.len())
}
