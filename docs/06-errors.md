# Errors

This page lists the errors the interpreter produces for any program, whatever families it uses: lexical errors,
block errors and dispatch errors. Each family's own errors are listed at the end of each statement on its
[statement page](05-statements/README.md).

## How an error is reported

The interpreter stops at the first error. It prints nothing the program would have printed; it prints the
message to standard error and exits with status 2. The `settle` command adds the program line the error points
at, with a caret under the place and its column:

```text
settle: line 5: settle does not take `sed:`; did you mean `seed:`?
   5 |   settle 100, sed: 1
     |               ^^^^ column 15
```

The first line is always `settle: line N: <message>`. The column is found from the message (`lex::locate`): the
first thing the message quotes in backticks that appears on that line, then the first `:symbol` it names; when
the message quotes neither, the caret underlines the whole statement. A message that runs over several lines (the
list of known statements) prints its first line, the excerpt, then the rest.

`N` is the 1-based line number in the program file. For an unclosed block, `N` is the line where the block was
opened. When the file cannot be read at all, the message is `settle: cannot read <path>: <reason>` and has no
line number. Two command-line mistakes are refused before any program is read, also with status 2:
`settle: unknown option <arg> (see settle --help)` and `settle: one program at a time; got <n> arguments (see
settle --help)` ([Install and run](01-install-and-run.md#the-command-line)), and `settle --json` with no program is
refused with `settle: --json needs a program (see settle --help)`. With `--json`, a program's error is printed as
a JSON object on standard output instead: its message, and the `line`, `column` and `width` the caret would mark.
When the library is used directly (`Interp::exec`), the error is a `SettleError` holding the text after
`settle: `, with no excerpt.

## Suggestions

Three errors suggest a correction when one is close: an unknown keyword (``did you mean `seed:`?``), an unknown
thing (`did you mean :sprinkler?`), and a line no family knows (``Did you mean `settle`?``, or, for a statement
written in the wrong kind of block, `` `settle` is a run statement; put it inside `run :name do ... end`. ``).
"Close" is at most one edit for a word of up to five letters and at most a third of the length for a longer
word, with a swap of two neighbouring letters counted as one edit and `-` and `_` counted as the same character
(so `warm-fit:` is pointed to `warm_fit:`). An edit never replaces the whole of the shorter word, so `:b` is not
corrected to `:a`. When no keyword is close, the keyword error lists every keyword the statement takes instead.

The examples on this page show that text, as the test in `tests/docs_examples.rs` records it.

## Lexical errors

These come from `src/words/lex.rs`, before a line is interpreted.

| Message | Cause |
|---|---|
| `a string is missing its closing quote` | A `"` with no second `"` on the same line. |
| `a ':' must start a symbol like :rain` | A `:` not followed by a letter, digit or underscore. |
| `'<text>' is not a number` | A token that starts like a number (`-`, a digit, or `.` then a digit) but does not parse, such as `-`, `1.2.3` or `1e`. |
| `unexpected '<character>'` | A character that starts no token, such as `(`, `=`, `+` or `[`. |

```settle example=err-string
model :m do
  thing :a
end

run :m do
  export "out.json, as: :ising
end
```

```text error=err-string
line 6: a string is missing its closing quote
```

## Block errors

These come from `src/words/interp.rs`.

| Message | Cause |
|---|---|
| `` blocks cannot nest; close the previous one with `end` `` | A `model :x do` or `run :x do` line inside an open block. |
| `` `end` without an open block `` | An `end` line when no block is open. |
| `` statements live inside `model :name do ... end` or `run :name do ... end` `` | A statement outside any block. |
| `no model :<name> to run` | `run :name do` for a model that has not been opened earlier in the file. |
| `` block :<name> is never closed with `end` `` | The file ended with a block open. Reported at the opening line. |

```settle example=err-nested
model :m do
  thing :a
run :m do
end
```

```text error=err-nested
line 3: blocks cannot nest; close the previous one with `end`
```

```settle example=err-unclosed
model :m do
  thing :a
```

```text error=err-unclosed
line 1: block :m is never closed with `end`
```

## No family knows the line

If no statement family claims a line, the error names the block kind and lists every statement form that the
registered families accept in that kind of block, one per line, from their help text:

```text
no statement family knows this line inside a model.<hint> Known:
    <every model statement form>
```

The hint is empty, or `` Did you mean `<verb>`? `` when the line's verb (`dance` in `dance :a`, `read` in
`s.read ...`) is close to a verb of that block kind, or `` `<verb>` is a run statement; put it inside `run :name
do ... end`. `` (and the same for model) when the verb belongs to the other kind of block.

The list is long and grows with every family, so the example below shows its full current text. Check the
statement's spelling and shape first: a statement with the right name but the wrong shape (for example
`hold :a` without a value) is also reported this way, because no family's pattern matches it.

```settle example=err-unknown-statement
model :m do
  thing :a
end

run :m do
  shake 100                        # no family knows `shake`
end
```

```text error=err-unknown-statement
line 6: no statement family knows this line inside a run. Known:
    hold :a, :yes
    settle 10_000, temperature: 1, seed: 1, update: :metro   (or :gibbs)
    anneal 4_000, seed: 1, update: :metro   (or :gibbs)
    show   /   best
    ask :a, and: :b, or_not: :c
    m.remember :cat   /   m.save :note, "some text"
    m.recall read-address: :cat, address-noise: 0.3, sweeps: 30, temperature: 0.1, seed: 1
    m.recall key: "secret"   (the key finds the memory and reads it)
    img.lean_from "frame.pgm", by: 1, correct: :yes
    img.show_as "out.pgm", from: :rate
    play :img, frames: "dir/", out: "dir2/", against: "other/", sweeps: 10, warm: :yes, read: :bits|:soft|:rb, keep: 1, correct: :tap|:mean|:bethe, copies: 8, update: :metro_checker|:gibbs|:checker|:metro|:cluster, fit: 8, fit_sweeps: 200, fit_update: :cluster, warm_fit: 1, warm_fit_sweeps: 400, warm_from: :correction|:leans, warm_step: 1, cut: 0.25, seed: 1, quiet: :no
    anneal_each 10_000, seed: 1, update: :metro|:gibbs   (each puzzle keeps its own calmest arrangement)
    s.solution   (after anneal: decode the calmest arrangement and check it by the rules)
    examples :data, "file.txt"   /   examples :data, rows: "110"
    learn :data, rounds: 200, rate: 0.05, method: :contrastive, sweeps: 1, batch: 50, decay: 0, seed: 1
    classify :test, labels: "d*", sweeps: 100, seed: 2
    shuffle :data, labels: "d*", seed: 3
    valleys show: 10   (exact, up to 24 free things)
    survey starts: 1000, sweeps: 50, temperature: 0.05, seed: 1, show: 8
    drift 20_000, step: 0.01, temperature: 1, seed: 1, burn: 2_000
    means   /   spread
    solve :x, :y, matrix: "2 1; 1 3", target: "1 2", steps: 100_000, step: 0.01
    s.write ... (as in a model)
    s.read read-address: :cat, address-noise: 0.3, iterated-reads: 10, via: :addresses, seed: 1   (via: :pulls too)
    s.read key: "secret"   /   s.read   (from noise)
    s.write :cat   /   s.write :note, "some text"
    s.read read-address: :cat, address-noise: 0.3, rounds: 3, samples: 16, mode: :pass, seed: 1   (mode :settle adds burn: 10)
    s.attend read-address: :cat, address-noise: 0.3, rounds: 3, seed: 1
    export "m.json", as: :ising   (also :qubo, :gset, :dimacs, :moments, :best)
    import "m.json"   (held things and temperature; refuses if the model differs)
    play_colour :film, frames: "dir/", out: "dir2/", sweeps: 20, warm: :yes, read: :soft, correct: :tap, copies: 1, against: "other/", seed: 1, quiet: :no
    k.read read-address: :cat, address-noise: 0.2, iterated-reads: 20, via: :addresses, seed: 1   (via: :pulls too, with wake: :fixed | :density | :top)
    m.recall_coded read-address: :note, address-noise: 0.3, knows: :name, sweeps: 30, temperature: 0.1, seed: 1   (knows :all = read-address from the stored pattern; sdm takes via: and iterated-reads:)
    d.train :train, rounds: 200, rate: 0.05, sweeps: 1, batch: 50, decay: 0, seed: 1, leans: :data
    d.generate 16, out: "samples.pgm", rows: "samples.txt", chain: "chain.pgm", sweeps: 100, seed: 2
    sample :train, 16, sweeps: 800, seed: 3, out: "direct.pgm", rows: "direct.txt"
    coins :train, 16, seed: 4, out: "coins.pgm", rows: "coins.txt"
    c.transmit flip: 0.03, seed: 1
    c.decode start: :received, sweeps: 400, hot: 1, cold: 0.05, seed: 2
    c.decode_bp
    c.decode_moves mover: :block, block: 4, sweeps: 400, seed: 2
    c.decode_nishimori mover: :block, sweeps: 2000, seed: 3
    refusal word-size: 256, load: 3000, level: 0.01   (the travel rule's threshold and the nearest-neighbour ceiling; no memory built)
    anneal_schedule 100_000, temperature: 2.9, hot: 10, cold: 0.05, restarts: 10, seed: 1, update: :metro|:gibbs
    f.final   (judge the end state the walk came to rest in, beside f.solution's best-so-far)
    contenttrack word-size: 256, hard-locations: 100000, load: 3000, address-noise: 0.3, block: 0, samples: 200   (TRACK-C's predicted recall for the content reads; no memory built)
    descend 20_000, step: 0.01, temperature: 1, cool_to: 0, batch: 100, walkers: 4, seed: 1, method: :adam
    ask   /   ask :w1, :bias   /   score
```

## Argument errors shared by every family

Every family reads its arguments with the same helpers in `src/words/lex.rs`, so these messages appear across the
language:

| Message | Cause |
|---|---|
| `` expected `key: value`, found `<token>` `` | Something other than `label value` pairs and commas where keyword arguments are expected. The token is quoted as written, for example `` `x` `` or `` `3` ``. |
| `` <statement> does not take `<key>:`; <hint> `` | A keyword the statement does not accept. The hint is `` did you mean `<key>:`? ``, or `` it takes `<a>:`, `<b>:` and `<c>:` ``, or `` <statement> takes no `key: value` arguments ``. |
| `a number was expected` | A keyword value that should be a number is not. |
| `a "quoted" string was expected` | A keyword value that should be a string is not. |
| `expected :yes or :no` | A keyword value that should be `:yes` or `:no` is not. |
| `unknown thing :<name> (declare it with: thing :<name>)` | A symbol that should name a declared thing does not. When a declared thing is close, the message is `unknown thing :<name>; did you mean :<thing>? (or declare it with: thing :<name>)`. |
| `<what> takes a whole number from <lo> to <hi>` | A count (sweeps, rounds, samples, a size) that is a fraction or out of range. Where a count has no upper limit the message is `<what> takes a whole number of <lo> or more; got <value>`. No family truncates a count: `show: 2.5` is refused, not read as 2. |
| `` `<statement>` is a KANERVA statement, and this SETTLE was built without the `sdm` feature; build it with the feature on (the default): cargo build --release --features sdm `` | An sdm-family statement (`memory`, `sdm`, `softsdm`, `sdmscale` in a model; `refusal`, `contenttrack` in a run) in a SETTLE built with `--no-default-features`. |

```settle example=err-bad-keyword
model :m do
  thing :a
end

run :m do
  settle 100, sweeps: 5            # settle takes temperature:, seed: and update: only
end
```

```text error=err-bad-keyword
line 6: settle does not take `sweeps:`; it takes `temperature:`, `seed:` and `update:`
```

```settle example=err-unknown-thing
model :m do
  thing :a
  a.pulls :b, by: 1                # :b was never declared
end
```

```text error=err-unknown-thing
line 3: unknown thing :b (declare it with: thing :b)
```

## Order errors

Some run statements read what an earlier statement produced, and refuse if it has not run in the same run
block. The core ones are `ask needs a settle first`, `show needs a settle first` and `best needs an anneal
first`; families add their own (for example a read before a write). Remember that each run block starts empty.

```settle example=err-ask-before-settle
model :m do
  thing :a
end

run :m do
  ask :a                           # nothing has been sampled yet
end
```

```text error=err-ask-before-settle
line 6: ask needs a settle first
```

## Refusals that are results

Some statements print a refusal as a normal output line instead of failing. They report that a result could not
be trusted: for example `solve` reports a system with no valley as not settled, a memory read can decline to
answer, and a puzzle's `solution` says NOT VALID. These are documented on the statement pages, and the program
continues after them.
