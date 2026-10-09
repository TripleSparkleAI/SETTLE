use settle::engine::model::Model;
use settle::lex::SettleError;

// the alarm program, written in Rust
fn alarm() -> Result<Vec<String>, SettleError> {
    Model::build()
        .thing("burglary").leans("no", 1.0)
        .thing("earthquake").leans("no", 1.0)
        .thing("alarm").leans("no", 1.0)
        .pulls("burglary", "alarm", 1.5)
        .pulls("earthquake", "alarm", 1.5)
        .pushes("burglary", "earthquake", 1.0)
        .run()
        .hold("alarm", "yes")
        .hold("earthquake", "yes")
        .settle(40_000).seed(1)
        .ask("burglary")
        .lines()
}
