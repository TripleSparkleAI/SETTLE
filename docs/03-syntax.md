# Syntax

This page describes what a SETTLE program may contain, token by token. It is derived from the lexer
(`src/lex.rs`) and the block handling in the interpreter (`src/interp.rs`). What each statement means is on the
[statement pages](05-statements/README.md).

## Lines

A program is UTF-8 text. The interpreter reads it one line at a time. **A statement is exactly one line**: there
is no line continuation and no statement separator. Each line is turned into a list of tokens, and a line with
no tokens (empty, only spaces, or only a comment) is skipped.

## Comments

`#` starts a comment that runs to the end of the line, except inside a string. There are no block comments.

## Tokens

There are seven kinds of token.

| Token | Written as | Examples |
|---|---|---|
| Symbol | `:` followed by a word | `:rain`, `:wet_grass`, `:yes`, `:ising` |
| Label | a word followed directly by `:` | `by:`, `temperature:`, `seed:` |
| Identifier | a word | `model`, `thing`, `settle`, `rain` |
| Number | digits, with optional `_`, `.`, `e`, and a leading `-` | `2`, `0.5`, `-1`, `20_000`, `1e3`, `.25` |
| String | text between double quotes | `"frame.pgm"`, `"1... .4.. ..4. ...1"` |
| Comma | `,` | |
| Dot | `.` not followed by a digit | the `.` in `rain.pulls` |

Spaces and tabs separate tokens and are otherwise ignored.

### Words

A word is a run of letters, digits and underscores. Letters and digits are Unicode letters and digits. An
identifier or label must start with a letter or an underscore. A symbol's word may start with a digit (`:1` is
a valid symbol), but it may not be empty: a lone `:` is an error.

### Labels and symbols

Whether a word followed by a colon is a label depends on the character after the colon:

- `by: 2` and `by:2` are the label `by` followed by the number `2`.
- `a::b` is not a label; the lexer reads the identifier `a` and then fails on the empty symbol `:`.
- A hyphen between a letter or digit and a letter joins one word: `read-address:` is one label and
  `:hard-locations` one symbol. A hyphen before a digit starts a number, so `by: -0.5` is unchanged.

### Numbers

A number starts with a digit, a `-`, or a `.` (a `.` counts as the start of a number only when a digit follows
it). The lexer then takes every following digit, `_`, `.` and `e`, removes the underscores, and parses the
result as a 64-bit floating point number. Consequences:

- `10_000` is ten thousand. Underscores may appear anywhere after the first character.
- `1e3` is one thousand, but an exponent cannot carry a sign. In `1e-3` the lexer stops at the `-`, and `1e`
  alone is the error `'1e' is not a number`. Write `0.001` instead.
- `-` on its own, or `1.2.3`, is an error: `'1.2.3' is not a number`.
- Every number is a floating point value. Statements that need a count (sweeps, sizes) truncate it towards zero.

### Strings

A string starts at `"` and ends at the next `"` on the same line. There are no escape sequences, so a string
cannot contain a double quote. A `#` inside a string is part of the string. A string with no closing quote is an
error. Strings are used for file paths and for inline data such as rows of bits.

## Blocks

Statements live inside blocks. A block opens with a line that contains exactly three tokens:

```text
model :name do
run :name do
```

and closes with a line that contains exactly the one token `end`.

- `model :name do` opens a model block. If no model with that name exists yet, an empty one is created.
  Opening a model block again with the same name adds to the same model.
- `run :name do` opens a run block for an existing model. The model must have been opened earlier in the
  file; otherwise the line is the error `no model :name to run`.
- Blocks cannot nest. A block-opening line inside an open block is an error.
- Every block must be closed. A block still open at the end of the file is an error that names the line where
  it opened.
- A statement outside any block is an error.

The words `model`, `run`, `do` and `end` are only special in these positions. A line such as `run :m, x: 1` is
not a block opening; it is offered to the statement families like any other line.

## Statements

Each line inside a block is a statement. The interpreter does not parse statements into a tree. It offers the
line's token list to each statement family in turn (see [Extending SETTLE](07-extending.md)), and the first
family whose pattern matches runs it. So the exact shape of each statement is defined by its family. In
practice every statement has one of two shapes:

```text
verb positional-arguments keyword-arguments
receiver.verb positional-arguments keyword-arguments
```

- The **verb** is an identifier: `thing`, `settle`, `ask`, `grid`, `play`.
- A **receiver** is the name of a thing or of a structure built by a statement, written as an identifier (no
  colon): `rain.pulls`, `img.show_as`, `m.recall`.
- **Positional arguments** come first: symbols, numbers or strings, separated by commas, in the order the
  statement defines.
- **Keyword arguments** come last: a label followed by one value token (`by: 2`, `seed: 1`, `as: :ising`). They
  may be separated by commas. Most statements accept their keywords in any order and refuse keywords they do
  not know, with the message `` <statement> does not take `<key>:` ``. A keyword retired for one of Kanerva's
  terms names its replacement instead: `` `cue:` is now `read-address:` (Kanerva's retrieval address) ``. A keyword given twice uses its first value.

Values of keyword arguments are single tokens. Where a statement needs a list (rows of bits, a matrix, the edges
of a graph), the list is written inside one string, and the statement parses the string itself; each statement
page gives the format.

### Yes and no

The symbols `:yes` and `:no` are the two values of a thing. They cannot be used as thing names.

## Example

```settle example=syntax-lexical
# A comment runs from # to the end of the line.
model :lexical do                  # a block opens with `model :name do`
  thing :first_thing, :second      # symbols start with a colon
  thing :second, leans: :no, by: 0.25
  first_thing.pulls :second, by: 1_000   # underscores may separate digits
  first_thing.pushes :second, by: 999.5  # a second pull on the same pair adds to the first
end

run :lexical do
  settle 1_000, temperature: 1.5, seed: 7
  show
end
```

```text output=syntax-lexical
settled: 1000 samples of 2 things at temperature 1.5
  first_thing    ############## 45.9%
  second         ############ 41.3%
```

The two lines about the pair add up to a pull of 0.5.

## Grammar

The grammar below describes the lexical structure exactly and the block structure exactly. The `statement` rule
gives the shape every family follows; which token sequences are valid statements is decided by the families.

```text
program      = { line } ;
line         = { blank } [ content ] { blank } [ comment ] newline ;
content      = block-open | block-close | statement ;

block-open   = ( "model" | "run" ) symbol "do" ;          (* exactly these three tokens *)
block-close  = "end" ;                                      (* exactly this one token *)

statement    = [ ident "." ] ident { argument } ;           (* shape only; see the family pages *)
argument     = [ "," ] ( symbol | number | string | ident | label value ) ;
value        = symbol | number | string | ident ;

symbol       = ":" word ;
label        = word-start { word-char } ":" ;              (* the ":" not followed by another ":" *)
ident        = word-start { word-char } ;
word         = word-char { word-char } ;
word-start   = letter | "_" ;
word-char    = letter | digit | "_" ;
number       = ( digit | "-" | "." digit ) { digit | "_" | "." | "e" } ;
                                                            (* "_" removed, then parsed as f64 *)
string       = '"' { any character except '"' } '"' ;
comment      = "#" { any character } ;
blank        = " " | tab ;
letter       = (* any Unicode alphabetic character *) ;
digit        = (* 0 to 9; Unicode digits inside words *) ;
```

## Lexical errors

These messages come from the lexer. Each is reported as `line N: <message>`.

- `a string is missing its closing quote`
- `a ':' must start a symbol like :rain`
- `'<text>' is not a number`
- `unexpected '<character>'` (for a character that starts no token, such as `(` or `=`)
- `` expected `key: value`, found <token> `` (a keyword argument list that contains something other than labels,
  values and commas; the token is shown in its internal form, such as `Ident("x")`)

The [Errors](06-errors.md) page lists every error, including block errors.
