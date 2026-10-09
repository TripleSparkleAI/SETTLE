# The zoo family

The zoo family writes hard puzzles as settling problems. Each puzzle statement declares a block of things and
sets their leans and pulls so that the calmest arrangement of the block is the answer to the puzzle. An anneal
then looks for that arrangement, and `x.solution` decodes the calmest arrangement found and checks it against
the puzzle's rules with plain code. The energy is never used for the verdict. Five puzzles are available:
sudoku, graph colouring, max-cut, factoring and nonograms.

The source is `src/zoo.rs`. The file `src/zoohard.rs` holds no statements: it has the exact solvers,
generators and deciders that the measurements used. The encodings and first measurements are in
`SETTLE/runs/settlezoo/REPORT_SETTLEZOO.md`. The column encoding for `factor` and the
`anneal_each` statement are in `SETTLE/runs/zoohard/REPORT_ZOOHARD.md`.

| Statement | Block | Summary |
|---|---|---|
| [`sudoku`](#sudoku) | model | declare a 4x4 or 9x9 sudoku |
| [`colouring`](#colouring) | model | declare a graph colouring with 2 to 9 colours |
| [`maxcut`](#maxcut) | model | declare a max-cut problem on a weighted graph |
| [`factor`](#factor) | model | declare the factoring of an odd whole number |
| [`nonogram`](#nonogram) | model | declare a nonogram: a picture drawn from row and column clues |
| [`anneal_each`](#anneal_each) | run | anneal, and let every puzzle keep its own calmest block |
| [`x.solution`](#xsolution) | run | decode a puzzle from the calmest arrangement and check it |

## How a puzzle becomes leans and pulls

Every puzzle except max-cut is first written as a QUBO: an energy over 0/1 variables `y`.

```text
E(y) = sum_i a_i y_i + sum_{i<j} b_ij y_i y_j + c
```

A price for each variable that is on, a price for each pair that is on together, and a constant.

Each variable becomes one thing, with `y = (1 + s) / 2`, where `s` is the thing's value, +1 (yes) or -1 (no).
The translation adds to the leans `h` and the pulls `J` of the model:

```text
h_i += -(a_i / 2 + sum_j b_ij / 4)        J_ij += -b_ij / 4
```

The model's energy then differs from the QUBO energy by a constant, so both have the same calmest arrangement.
A positive pair price becomes a push.

Most rules are "exactly one of these variables is on". The zoo writes such a rule as a penalty:

```text
A (sum_{v in group} y_v - 1)^2
```

It is zero when exactly one member of the group is on, and at least `A` otherwise.

Each puzzle records where its things are in the model's notes, under the key `zoo:<name>`. Several puzzles can
therefore share one model. A puzzle name must be unique within a model.

## `sudoku`

**Block:** model.

**Form:**

```text
sudoku :name, size: 9, given: "....", by: 1, given_by: 4
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `:name` | symbol | required | the puzzle's name, used by `name.solution` and in thing names |
| `size:` | number, 4 or 9 | `9` | the side of the grid |
| `given:` | string | every cell empty | the givens, row by row |
| `by:` | number | `1` | the penalty `A` for each broken "exactly one" rule; must be above zero |
| `given_by:` | number | `4 * by` | the lean `G` toward each given digit; must be above zero |

**What it does:** declares `size * size * size` things, one for each pair of a cell and a digit. The thing
`name_r<row>c<col>_<digit>` is yes when that cell holds that digit. Rows, columns and digits count from 1. The
things are added in the order row, then column, then digit.

The `given:` string holds `size * size` characters once all whitespace is removed, so spaces between groups
are only for reading. A `.` or a `0` is an empty cell. A digit from 1 to `size` is a given.

The energy is:

```text
E = A * sum_groups (sum_{v in group} y_v - 1)^2  -  G * sum_givens y_(cell, given digit)
```

The groups are: the digits of each cell, and each digit within each row, each column and each box. The box
side is the square root of `size` (2 or 3). The given term leans each given cell toward its digit with strength
`G`. The default `G = 4A` exists because at `G = A` a test anneal settled into a valid grid that dropped one
given: dropping a given costs only `G`, while moving between two valid grids crosses a high ridge.

The layout is stored in the note `zoo:<name>`: the index of the first thing, the size, the word `sudoku` and
the givens.

**Output:** none.

**Example:**

```settle example=zoo-sudoku
# A 4x4 sudoku with four givens. Rows are separated by spaces; '.' is an empty cell.
model :p do
  sudoku :s, size: 4, given: "1... .4.. ..4. ...1"
end

run :p do
  anneal 2_000, seed: 1   # look for the calmest arrangement
  s.solution              # decode it into a grid and check every rule
end
```

Output:

```text output=zoo-sudoku
annealed: 2000 sweeps, calmest energy found -136.000
  1 2 | 3 4
  3 4 | 1 2
  ---------
  2 1 | 4 3
  4 3 | 2 1
sudoku :s: VALID (checked rule by rule, not by energy)
```

**Errors:**

- `puzzle :<name> is already declared`
- `` sudoku does not take `<key>:` ``
- `sudoku size is 4 or 9`
- `by: and given_by: must be above zero`
- `a <n>x<n> sudoku needs <n*n> cells in given:, found <count>`
- `'<char>' is not a digit from 1 to <n> or '.'`
- `a number was expected` (for `size:`, `by:` or `given_by:`)
- `a "quoted" string was expected` (for `given:`)

## `colouring`

**Block:** model.

**Form:**

```text
colouring :name, colours: 3, edges: "a-b b-c c-a", by: 1
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `:name` | symbol | required | the puzzle's name |
| `colours:` | whole number from 2 to 9 | `3` | how many colours may be used |
| `edges:` | string | required | the graph's edges, like `"a-b b-c c-a"` |
| `by:` | number | `1` | the penalty for a node without exactly one colour, and for an edge whose ends share a colour |

**What it does:** reads the edge list and declares one thing for each pair of a node and a colour. The thing
`name_<node>_<colour>` is yes when that node has that colour. Colours count from 1. Nodes are numbered in the
order they first appear in `edges:`, and the things are added node by node, colour by colour.

The edge list is a string of edges separated by whitespace. Each edge is two node names joined by `-`. A node
name is letters, digits and `_`. An edge may carry a weight after a colon, as in `a-c:2`. `colouring` reads the
weight but does not use it. An edge that appears twice adds its penalty twice.

The energy is:

```text
E = A * sum_nodes (sum_c y_(u,c) - 1)^2  +  B * sum_edges sum_c y_(u,c) y_(v,c)
```

The first term asks for exactly one colour per node. The second costs `B` for every edge `u-v` and colour `c`
where both ends have colour `c`. Both `A` and `B` are set by `by:`. The code does not check that `by:` is above
zero.

The layout is stored in the note `zoo:<name>`: the first thing, the number of colours, the word `colouring` and
the edge list.

**Output:** none.

**Example:** two colourings of one triangle. Three colours are enough; two are not.

```settle example=zoo-colouring
# A triangle needs three colours. Two colourings of the same triangle share one model.
model :p do
  colouring :three, colours: 3, edges: "a-b b-c c-a"
  colouring :two, colours: 2, edges: "a-b b-c c-a"
end

run :p do
  anneal_each 1_000, seed: 1   # each puzzle keeps its own calmest block
  three.solution               # three colours: a proper colouring exists
  two.solution                 # two colours: no proper colouring exists
end
```

Output:

```text output=zoo-colouring
annealed each: 1000 sweeps, calmest energy found -7.250; per puzzle :three -5.250, :two -2.000
  (:three judged on its own calmest arrangement from anneal_each)
  a 1, b 2, c 3
colouring :three with 3 colours: PROPER (checked edge by edge, not by energy)
  (:two judged on its own calmest arrangement from anneal_each)
  a 2, b 1, c -
colouring :two with 2 colours: NOT PROPER: node c has no colour
```

**Errors:**

- `puzzle :<name> is already declared`
- `` colouring does not take `<key>:` ``
- `colours must be from 2 to 9`
- `needs edges: "a-b b-c ..."`
- `edges: needs at least one edge like "a-b"`
- `'<item>' is not an edge like a-b`
- `'<name>' in '<item>' is not a node name`
- `'<item>' joins a node to itself`
- `'<item>' has a weight that is not a number`

## `maxcut`

**Block:** model.

**Form:**

```text
maxcut :name, edges: "a-b b-c:2 c-a", target: 3
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `:name` | symbol | required | the puzzle's name |
| `edges:` | string | required | the graph's edges, with an optional `:weight` (default 1) after each |
| `target:` | number | none | a cut value that `name.solution` reports as reached or not reached |

**What it does:** declares one thing per node, named `name_<node>`, in the order the nodes first appear in
`edges:`. The edge list has the same form as for [`colouring`](#colouring). Every edge `u-v` with weight `w`
adds a pull of `-w` between its two ends, that is, a push. The energy is:

```text
E = sum_edges w * s_u * s_v
```

An edge whose ends are on opposite sides (one yes, one no) lowers the energy by `w`, so the calmest arrangement
is a split of the nodes that cuts the largest total weight. No QUBO is needed.

The layout is stored in the note `zoo:<name>`: the first thing, the target (or no target), the word `maxcut`
and the edge list.

**Output:** none.

**Example:** a square with one diagonal of weight 2, and a triangle with a target that no split can reach.

```settle example=zoo-maxcut
# A square with one weighted diagonal, and a triangle with a target it cannot reach.
model :p do
  maxcut :sq, edges: "a-b b-c c-d d-a a-c:2"   # a-c has weight 2
  maxcut :tri, edges: "a-b b-c c-a", target: 3
end

run :p do
  anneal_each 1_000, seed: 2
  sq.solution    # the two sides, the cut, and the brute-force best
  tri.solution   # a triangle's best cut is 2, so a target of 3 is not reached
end
```

Output:

```text output=zoo-maxcut
annealed each: 1000 sweeps, calmest energy found -3.000; per puzzle :sq -2.000, :tri -1.000
  (:sq judged on its own calmest arrangement from anneal_each)
  side yes: c   side no: a b d
maxcut :sq: cut 4, exact best 4 by brute force: OPTIMAL
  (:tri judged on its own calmest arrangement from anneal_each)
  side yes: b   side no: a c
maxcut :tri: cut 2, exact best 2 by brute force: OPTIMAL, target 3: NOT REACHED
```

**Errors:**

- `puzzle :<name> is already declared`
- `` maxcut does not take `<key>:` ``
- `needs edges: "a-b b-c ..."`
- the edge list errors listed under [`colouring`](#colouring)
- `a number was expected` (for `target:`)

## `factor`

**Block:** model.

**Form:**

```text
factor :name, number: 10_403, encoding: :columns, penalty: 2
factor :name, number: 143, encoding: :rosenberg, penalty: 128
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `:name` | symbol | required | the puzzle's name |
| `number:` | odd whole number | required | the number `N` to factor: 9 to 1,000,000 with `:rosenberg`, 9 to 10^12 with `:columns` |
| `encoding:` | one of `:columns` / `:rosenberg` | `:columns` (`:rosenberg` until 2026-10-06) | how the multiplication is written as leans and pulls; `:columns` solves more (below) |
| `penalty:` | number | `2^(pb + qb - 2)` with `:rosenberg`, `2` with `:columns` | the weight `λ` that holds each helper to its product |

**What it does:** looks for two odd factors `p` and `q` of `N`, written in binary. The bit widths are:

```text
pb = max(bitlen(floor(sqrt(N))), 2)        qb = bitlen(N) - pb + 1
```

`p` is the smaller factor, at most the square root of `N`, so the trivial split `1 x N` cannot fit. The lowest
bit of each factor is fixed at 1, so both factors are odd. The statement declares these things, in this order:

| Things | Meaning |
|---|---|
| `name_p1` .. `name_p<pb-1>` | the free bits of `p` |
| `name_q1` .. `name_q<qb-1>` | the free bits of `q` |
| `name_z<i>_<j>` for every `i` from 1 to `pb-1` and `j` from 1 to `qb-1` | a helper for the product `p_i * q_j` |
| `name_c<k>_<m>` (`:columns` only) | the carry bits of column `k` of the long multiplication |

A machine with only pair pulls cannot multiply three bits together, so each product of two free bits is
replaced by a helper `z`. The Rosenberg penalty holds each helper to its product:

```text
λ (3z + p_i q_j - 2 p_i z - 2 q_j z)
```

It is zero when `z = p_i * q_j` and at least `λ` otherwise.

With `encoding: :rosenberg`, the energy is the squared error of the whole product plus the helper penalties:

```text
E = (N - p*q)^2 + λ * sum_helpers (3z + p_i q_j - 2 p_i z - 2 q_j z)
```

where `p*q` is written through the helpers as `sum 2^(i+j) z_ij` plus the terms that involve bit 0. The pulls
of this form grow like `N^2`.

With `encoding: :columns`, each column `k` of the long multiplication is squared on its own, for `k` from 1 to
`bitlen(N) - 1`:

```text
E = sum_k (sum_{i+j=k} p_i q_j + carries into k - N_k - sum_m 2^m c_(k,m))^2 + helper penalties
```

`N_k` is bit `k` of `N`. A carry `c_(k,m)` leaves column `k` with weight `2^m` and lands in column `k + m` with
weight 1. Each column gets just enough carry bits to hold the largest carry it can produce; carries that would
land above the top column are not created. Every term is at least zero, and the energy is zero exactly at a
factorisation with its carries, for any `λ` above zero. The pulls of this form stay within a factor of about 50
of each other.

Measured over 50 seeds with 100,000 sweeps, each encoding at a temperature of its largest pull divided by 10
(`runs/zoohard/measure_factor.txt`), `:columns` solved 899 in 100% of runs against 14% for `:rosenberg`, and
3,599 in 74% against 0%. `:columns` is the default since 2026-10-06; write `encoding: :rosenberg`
for the old form. The anneal's temperature still matters: the zoo's rule is the largest pull divided by 10, which
is 0.55 for 143 in the column encoding and 1158.4 in the Rosenberg one.

The layout is stored in the note `zoo:<name>`: the first thing, `N`, `pb`, `qb`, the number of things, the word
`factor` and the encoding.

**Output:** none.

**Example:** 15 with the default (column) encoding and 21 with the Rosenberg encoding, in one model.

```settle example=zoo-factor
# Factor 15 with the default encoding (columns, since 2026-10-06), and 21 with the older Rosenberg encoding.
model :p do
  factor :f, number: 15
  factor :g, number: 21, encoding: :rosenberg
end

run :p do
  anneal_each 2_000, seed: 1
  f.solution   # p x q, checked by multiplying
  g.solution
end
```

Output:

```text output=zoo-factor
annealed each: 2000 sweeps, calmest energy found -178.500; per puzzle :f -4.500, :g -174.000
  (:f judged on its own calmest arrangement from anneal_each)
factor :f: 15 = 3 x 5: VALID (checked by multiplying)
  (:g judged on its own calmest arrangement from anneal_each)
factor :g: 21 = 3 x 7: VALID (checked by multiplying)
```

**Errors:**

- `puzzle :<name> is already declared`
- `` factor does not take `<key>:` ``
- `encoding is :columns (the default) or :rosenberg`
- `factor needs number:`
- `factor takes an odd whole number from 9 to 10^12`
- `factor with encoding: :rosenberg takes an odd whole number from 9 to 1,000,000`

## `nonogram`

**Block:** model.

**Form:**

```text
nonogram :name, rows: "1 1/5/5/3/1", cols: "2/4/4/4/2", by: 1
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| `:name` | symbol | required | the puzzle's name |
| `rows:` | string | required | the row clues, top to bottom: rows separated by `/`, the run lengths of one row by spaces; `0` or nothing is an empty row |
| `cols:` | string | required | the column clues, left to right, in the same form |
| `by:` | number | `1` | the price of each broken rule; above zero |

**What it does:** declares one thing per cell, named `name_r<row>c<column>`, yes for a filled cell. For every
block of a row or column clue it also declares a row of "past here" things, `name_row<i>_b<k>_past<p>` (and
`name_col<i>_b<k>_past<p>` for a column), one for every place the block could start except the first: thing
`past<p>` is yes when the block starts after cell `p` of its line. A valid start reads
yes, yes, ..., no, no (a single change from yes to no), so moving a block by one cell is one flip. The costs, each
of price `by:`: one change per block, the blocks of a line in order with at least one empty cell between them, and
each cell equal to the blocks of its row that cover it, and to the blocks of its column that cover it. The energy
is zero exactly at a picture whose every row and column reads back as its clue, and the encoding goes through the
QUBO translation above. It is the same encoding as the browser nonogram on the SETTLE site (`#/puzzles`).

The layout is stored in the note `zoo:<name>`: the first thing, the height, the width, the number of variables,
the word `nonogram`, and the two clue strings.

**Output:** none.

**Example:** a heart, and a puzzle whose row clues ask for three filled cells while the column clues ask for two.

```settle example=zoo-nonogram
# A 5x5 nonogram that draws a heart, and a 3x3 one whose clues no picture satisfies.
model :p do
  nonogram :heart, rows: "1 1/5/5/3/1", cols: "2/4/4/4/2"
  nonogram :odd, rows: "1/1/1", cols: "1/1/0"   # three cells by the rows, two by the columns
end

run :p do
  anneal_each 4_000, seed: 1
  heart.solution    # the picture, then every row and column checked against its clue
  odd.solution      # the calmest arrangement cannot fit clues that disagree
end
```

Output:

```text output=zoo-nonogram
annealed each: 4000 sweeps, calmest energy found -43.750; per puzzle :heart -32.000, :odd -11.750
  (:heart judged on its own calmest arrangement from anneal_each)
  .#.#.
  #####
  #####
  .###.
  ..#..
nonogram :heart: VALID (every row and column read back against its clue, not by energy)
  (:odd judged on its own calmest arrangement from anneal_each)
  #..
  .#.
  ...
nonogram :odd: NOT VALID: row 3 reads [], its clue is [1]
```

**Errors:**

- `puzzle :<name> is already declared`
- `` nonogram does not take `<key>:` ``
- `nonogram needs rows: "1 1/5/5/3/1"` (also `cols:`)
- `'<text>' is not a run length`
- `row <i> clue does not fit in <n> cells` (also `column`)
- `by: must be above zero`
- `a "quoted" string was expected`, `a number was expected`

## `anneal_each`

**Block:** run.

**Form:**

```text
anneal_each 10_000, temperature: 1, seed: 1, update: :metro
```

**Arguments:**

| Argument | Type | Default | Meaning |
|---|---|---|---|
| sweeps | whole number | required | how many sweeps the anneal takes |
| `temperature:` | number | the run's temperature (1 at the start of a run) | the base temperature; sets the run's temperature; must be above zero |
| `seed:` | whole number | the run's random stream | reseeds the run's random stream before the anneal |
| `update:` | `:metro` or `:gibbs` | the run's current rule (`:metro` at the start of a run) | the rule each sweep updates a thing by, as for the core [`settle`](core.md#settle) |

**What it does:** walks exactly the path the core `anneal` statement walks, with the same schedule and the same
random draws, so for the same seed it visits the same arrangements. The temperature at step `k` of `S` sweeps
is:

```text
T_k = 10 * T * 0.005^(k / (S - 1))
```

It starts at ten times the run's temperature and cools to one twentieth of it. Held things stay held. Like
`anneal`, it keeps the calmest whole-model arrangement it visits as the run's best, and leaves the run's last
arrangement at the final step.

In addition, every puzzle declared in the model keeps the block of its own things that was calmest for its own
energy. A puzzle's own energy counts its leans and the pulls inside its block only. These blocks are stored in
the notes under `zoo:each:<name>`, stamped with a fingerprint of the run's best arrangement. When puzzles share
a model, a plain `anneal` judges every puzzle on the one arrangement that was calmest for the sum of all of
them. With `anneal_each`, each puzzle is judged on its own best block instead.

A later plain `anneal` replaces the run's best, the stamp no longer matches, and `x.solution` falls back to the
whole-model best.

**Output:**

```text
annealed each: <sweeps> sweeps, calmest energy found <energy>; per puzzle :<name> <energy>, :<name> <energy>
```

The whole-model energy and each puzzle's own energy have three decimals. Puzzles are listed in name order.

**Example:** see the examples for [`colouring`](#colouring), [`maxcut`](#maxcut) and [`factor`](#factor).

**Errors:**

- `` anneal_each does not take `<key>:` ``
- `` `update:` takes :gibbs or :metro ``
- `temperature must be above zero`

## `x.solution`

**Block:** run.

**Form:**

```text
name.solution
```

**Arguments:** none. `name` is a puzzle declared with `sudoku`, `colouring`, `maxcut`, `factor` or `nonogram`. A line
`x.solution` where `x` is not a declared puzzle is not claimed by this family.

**What it does:** takes the run's best arrangement from the last `anneal` or `anneal_each`. If the last anneal
was `anneal_each` and its stamp still matches, the puzzle's own calmest block replaces the puzzle's part of that
arrangement. Then it decodes the puzzle's things (yes means on) and checks the result with plain code:

- **sudoku:** each cell gets the digit whose thing is on. A cell with no digit on prints `.`, and a cell with
  several prints `?`. The check fails at the first of: a cell with no digit, a cell with several digits, a given
  that was changed, a row, column or box that holds a digit twice. Cells are checked first, then row 1,
  column 1, box 1, row 2, and so on.
- **colouring:** each node gets the colour whose thing is on, `-` for none and `?` for several. The check fails
  at the first node without exactly one colour, then at the first edge whose ends share a colour.
- **maxcut:** the yes things form one side and the no things the other. It adds up the weight of the edges that
  cross. For 22 nodes or fewer it also finds the best cut by trying every split, and says whether the found cut
  reaches it. If `target:` was given, it says whether the cut reaches the target.
- **factor:** reads `p` and `q` from the `p` and `q` bits (bit 0 is always 1) and multiplies them. The check
  fails if either factor is below 2 or if `p * q` is not `N`. The helper and carry things are not read.
- **nonogram:** each cell is filled when its thing is yes. Every row, then every column, is read back as run
  lengths and compared with its clue. The "past here" things are not read.

It changes nothing in the run.

**Output:** if the puzzle was judged on its own block from `anneal_each`, the first line is:

```text
  (:<name> judged on its own calmest arrangement from anneal_each)
```

Then, for a sudoku, the grid, one line per row, with `| ` between boxes and a line of dashes between bands of
boxes, followed by one of:

```text
sudoku :<name>: VALID (checked rule by rule, not by energy)
sudoku :<name>: NOT VALID: <first broken rule>
```

For a colouring:

```text
  <node> <colour>, <node> <colour>, ...
colouring :<name> with <k> colours: PROPER (checked edge by edge, not by energy)
colouring :<name> with <k> colours: NOT PROPER: <first broken rule>
```

For a max-cut, where the parts in brackets appear only when they apply:

```text
  side yes: <nodes>   side no: <nodes>
maxcut :<name>: cut <cut>[, exact best <best> by brute force: OPTIMAL | SHORT by <gap>][, target <t>: REACHED | NOT REACHED]
```

For a factoring, with the smaller factor first:

```text
factor :<name>: <N> = <p> x <q>: VALID (checked by multiplying)
factor :<name>: NOT VALID for <N>: <p> x <q> = <product>, not <N>
factor :<name>: NOT VALID for <N>: <p> x <q> uses a trivial factor
```

For a nonogram, the picture, one line per row, `#` for a filled cell and `.` for an empty one, then:

```text
nonogram :<name>: VALID (every row and column read back against its clue, not by energy)
nonogram :<name>: NOT VALID: row <k> reads [<runs>], its clue is [<runs>]
```

The column message has the same form with `column`.

The broken-rule messages are: `cell r<r>c<c> has no digit`, `cell r<r>c<c> has several digits`,
`cell r<r>c<c> was given <d> but holds <e>`, `row <k> has two <d>s` (also `column` and `box`),
`node <u> has no colour`, `node <u> has several colours`, `edge <u>-<v> has colour <c> at both ends`.

**Example:** see the examples above. This one shows the error a program meets when it asks for a solution
before any anneal:

```settle example=zoo-solution-first
# x.solution reads the calmest arrangement of an anneal, so it needs one first.
model :p do
  factor :f, number: 15
end

run :p do
  f.solution   # no anneal has run yet
end
```

```text error=zoo-solution-first
line 7: solution needs an anneal first
```

**Errors:**

- `solution needs an anneal first` (a `settle` does not count; only `anneal` and `anneal_each` keep a best)

## Notes

- A run starts with an empty best. Each `run` block starts a fresh run, so the anneal and `x.solution` must be
  in the same `run` block.
- An anneal only finds the calmest arrangement with some chance. The more sweeps, the higher the chance. On a
  4x4 sudoku with four givens, 2,000 sweeps succeeded in 100% of 50 seeds; a 9x9 puzzle with 30 givens needed
  about 50,000 sweeps for 94% (`runs/settlezoo/REPORT_SETTLEZOO.md`). Harder 9x9 puzzles and larger graphs are
  measured in `runs/zoohard/REPORT_ZOOHARD.md`.
- Because `x.solution` checks the decoded answer with plain rules, a NOT VALID verdict is always honest, even for
  a puzzle with no answer, such as a prime passed to `factor`.
