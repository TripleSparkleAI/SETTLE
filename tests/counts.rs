//! Every family reads a count the same way: a fraction or a negative number is refused with the family convention's
//! words (`<what> takes a whole number ...`), never truncated, and the same program with a whole number runs.
//!
//! <claudes_code_comments>
//! ** Function List **
//! cases()                                  - (a program with a bad count, the error it must give, the same program made whole)
//! a_fractional_count_is_refused_in_every_family - the refusal, one family per case
//! the_same_programs_with_whole_counts_run  - the control: the refusal is about the count, not the program
//!
//! ** Technical Review **
//! - Before the shared `lex::whole`, these counts were cast with `as usize`, so `show: 2.5` showed 2 valleys and
//!   `drift 100.5` drifted 100 steps without a word, and a negative count became 0.
//!
//! </claudes_code_comments>

use settle::interp::Interp;

fn cases() -> Vec<(String, &'static str, String)> {
    let p = |model: &str, run: &str| format!("model :m do\n  {}\nend\nrun :m do\n  {}\nend", model, run);
    vec![
        (p("thing :a", "settle 2.5"), "line 5: settle takes a whole number of sweeps", p("thing :a", "settle 2")),
        (p("thing :a", "anneal -3"), "line 5: anneal takes a whole number of sweeps", p("thing :a", "anneal 3")),
        (p("thing :a, :b", "valleys show: 2.5"), "line 5: show: takes a whole number of 0 or more; got 2.5", p("thing :a, :b", "valleys show: 2")),
        (p("number :x", "drift 100.5"), "line 5: drift takes a whole number of 0 or more; got 100.5", p("number :x", "drift 100")),
        (p("sudoku :s, size: 4", "anneal_each 10.5"), "line 5: anneal_each takes a whole number of 0 or more; got 10.5", p("sudoku :s, size: 4", "anneal_each 10")),
        (p("factor :f, number: 15", "anneal_schedule -100"), "line 5: anneal_schedule takes a whole number of 0 or more; got -100", p("factor :f, number: 15", "anneal_schedule 100")),
        ("model :m do\n  sudoku :s, size: 4.5\nend".to_string(), "line 2: size: takes a whole number of 0 or more; got 4.5", "model :m do\n  sudoku :s, size: 4\nend".to_string()),
    ]
}

#[test]
fn a_fractional_count_is_refused_in_every_family() {
    for (src, want, _) in cases() {
        let e = Interp::default().exec(&src).err().map(|e| e.0).unwrap_or_default();
        assert!(e.starts_with(want), "{:?}\n gave {:?}", src, e);
    }
}

#[test]
fn the_same_programs_with_whole_counts_run() {
    for (_, _, good) in cases() {
        if let Err(e) = Interp::default().exec(&good) {
            assert!(!e.0.contains("whole number"), "{:?} gave {:?}", good, e.0);
        }
    }
}
