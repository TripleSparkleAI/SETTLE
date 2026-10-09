# The science

This page states the mathematics each part of the interpreter implements, with a plain reading of each equation
and a reference to where the idea comes from. The implementation details (defaults, output, edge cases) are on
the [statement pages](05-statements/README.md); this page explains why the statements do what they do.

Notation: `s_i` is the value of thing `i`, +1 for yes and -1 for no. `h_i` is its lean, `J_ik` the pull between
things `i` and `k`, and `T` the temperature.

## The Ising energy

A SETTLE model is an Ising model with arbitrary leans (fields) and pulls (couplings):

```text
E(s) = - sum_i h_i s_i  -  sum_{i<k} J_ik s_i s_k
```

Reading: a thing that points the way it leans lowers the energy by its lean, and a pair that agrees lowers the
energy by its pull (a negative pull, written `pushes`, rewards disagreement instead).

The model is named after Ernst Ising, who solved its one-dimensional case. Implemented in `Model::energy`
(`src/engine/model.rs`).

- E. Ising, "Beitrag zur Theorie des Ferromagnetismus", *Zeitschrift fur Physik* 31, 253-258 (1925).

## The Boltzmann distribution

Settling draws arrangements from

```text
P(s) = exp(-E(s) / T) / Z,      Z = sum over all s of exp(-E(s) / T)
```

Reading: calm arrangements are exponentially more likely than excited ones, and the temperature sets how sharply
the probability concentrates on the calmest. `ask` estimates probabilities under this distribution, conditioned
on any held things.

- J. W. Gibbs, *Elementary Principles in Statistical Mechanics* (Yale University Press, 1902).

## Gibbs sampling, Glauber dynamics and p-bits

`settle` updates one free thing at a time. The input to thing `i` is `I_i = h_i + sum_k J_ik s_k`, and the thing is
set to yes with probability

```text
P(s_i = +1 | all other things) = exp(I_i / T) / (exp(I_i / T) + exp(-I_i / T)) = (1 + tanh(I_i / T)) / 2
```

Reading: this is the exact probability of `s_i` given everything else under the Boltzmann distribution, so
updating things this way, over and over, leaves the distribution unchanged and in the long run visits
arrangements in proportion to their probability.

A thing updated by this rule is what the hardware literature calls a p-bit: a probabilistic bit whose average
follows `tanh` of its input. SETTLE's implementation compares `tanh(I_i / T)` with a uniform random number in
[-1, 1), which is the p-bit rule and is exactly Gibbs sampling. A sweep visits the free things in a fresh random
order each time (`State::sweep`).

- R. J. Glauber, "Time-dependent statistics of the Ising model", *Journal of Mathematical Physics* 4, 294-307 (1963).
- S. Geman and D. Geman, "Stochastic relaxation, Gibbs distributions, and the Bayesian restoration of images",
  *IEEE Transactions on Pattern Analysis and Machine Intelligence* 6(6), 721-741 (1984).
- K. Y. Camsari, R. Faria, B. M. Sutton and S. Datta, "Stochastic p-bits for invertible logic",
  *Physical Review X* 7, 031014 (2017).

### Estimates from correlated samples

Consecutive sweeps are correlated, so `N` recorded samples carry less information than `N` independent draws.
With an integrated autocorrelation time `tau` (in sweeps), the variance of an average over `N` samples is about

```text
Var(average) ~= (2 tau / N) Var(single sample)
```

Reading: a chain that forgets its past slowly behaves like a smaller sample; strong pulls, low temperatures and
deep separated valleys all make `tau` larger. The FILMSHARP and FILMWARM experiments measure `tau` for the grid
(`examples/filmsharp_tau.rs`, `examples/filmwarm_tau.rs`).

## Simulated annealing

`anneal` samples while lowering the temperature, sweep `k` of `N` at

```text
T_k = 10 T x 0.005^( k / (N - 1) )
```

and keeps the lowest-energy arrangement visited. Reading: at a high temperature the sampler moves freely between
valleys; as the temperature falls it settles into a deep one. A slow enough schedule finds the global minimum
with high probability, but a finite schedule gives no guarantee. Measured on factoring, the walk often passes the
answer and then leaves it: factoring 899 in 50,000 sweeps, 90% of 100 walks visited the answer and 9% ended in it
(`runs/zoohard/measure_dwave.txt`). That is why `anneal` keeps the calmest arrangement visited, and why
`x.final` reports the end state separately.

- S. Kirkpatrick, C. D. Gelatt and M. P. Vecchi, "Optimization by simulated annealing", *Science* 220(4598),
  671-680 (1983).

## Hopfield memory

The memory family stores patterns `xi^mu` (vectors of +1 and -1) in the pulls by the Hebbian rule

```text
J_ik = (1 / n) sum_mu xi_i^mu xi_k^mu
```

where `n` is the memory's size. Reading: pairs that agree in a stored pattern are pulled together and pairs that
disagree are pushed apart, so each stored pattern (and its mirror image) becomes a valley, and settling at a low
temperature from a noisy read-address rolls back into it. A memory of `n` things holds about `0.14 n` random patterns
before the valleys merge.

- J. J. Hopfield, "Neural networks and physical systems with emergent collective computational abilities",
  *Proceedings of the National Academy of Sciences* 79(8), 2554-2558 (1982).
- D. J. Amit, H. Gutfreund and H. Sompolinsky, "Storing infinite numbers of patterns in a spin-glass model of
  neural networks", *Physical Review Letters* 55, 1530-1533 (1985).

## Sparse distributed memory

The sdm, softsdm, sdmscale and sdmrefuse families implement Kanerva's sparse distributed memory. `M` hard
locations have fixed random addresses and a row of bit-counters each. Writing pattern `p` adds `p` to the bit-counters of
every hard location whose address is within activation radius `r` of `p`. Reading from read-address `z` sums the bit-counters of the
hard locations within activation radius `r` of `z` and takes the sign:

```text
z_j <- sign( sum over { i : d(a_i, z) <= r } of C_ij )
```

Reading: many hard locations each hold a blurred superposition of the patterns written near them, and the majority over
the hard locations near a read-address reconstructs the pattern nearest that read-address. Iterating the read moves a noisy read-address towards
the stored pattern. The sdm page explains how the bit-counters become SETTLE pulls. The sdmscale and sdmrefuse experiments use
Bricken and Pehlevan's analysis to choose the activation radius and to predict when a read converges.

SETTLE names every part with Kanerva's own word, joined by a hyphen where his word is two: `read-address:`,
`address-noise:`, `hard-locations:`, `activation-radius:`, `activation-probability:`, `iterated-reads:` and
`word-size:` are keywords, and write-address, access-circle, data-word, bit-counters, read-threshold,
critical-distance and best-match are the words the pages use. Each one, with the sentence Kanerva wrote it in and
its page, is in `SETTLE/kanerva/KANERVA_TERMS.md`.

- P. Kanerva, *Sparse Distributed Memory* (MIT Press, 1988).
- P. Kanerva, "Sparse distributed memory and related models", in M. H. Hassoun (ed.), *Associative Neural
  Memories: Theory and Implementation*, Oxford University Press (1993), pp. 50-76.
- T. Bricken and C. Pehlevan, "Attention approximates sparse distributed memory", *Advances in Neural Information
  Processing Systems* 34 (2021), arXiv:2111.05498.

## Boltzmann machine learning

The learn and denoise families fit leans and pulls so that the model's distribution matches a set of examples.
The log-likelihood gradient for a pull is

```text
d log L / d J_ik = < s_i s_k >_data - < s_i s_k >_model
```

and for a lean `< s_i >_data - < s_i >_model`. Reading: raise a pull when the examples agree on that pair more
often than the settled model does, and lower it when less often. Hidden things, which the examples do not specify,
let the model represent structure that visible pulls alone cannot. The four `method:` values of `learn` estimate
the model average in different ways: exactly by enumeration, by contrastive divergence (a short settle started at
each example), by persistent chains, or by maximising the pseudo-likelihood instead.

- D. H. Ackley, G. E. Hinton and T. J. Sejnowski, "A learning algorithm for Boltzmann machines",
  *Cognitive Science* 9(1), 147-169 (1985).
- G. E. Hinton, "Training products of experts by minimizing contrastive divergence", *Neural Computation* 14(8),
  1771-1800 (2002).
- T. Tieleman, "Training restricted Boltzmann machines using approximations to the likelihood gradient",
  *Proceedings of the 25th International Conference on Machine Learning*, 1064-1071 (2008).
- J. Besag, "Statistical analysis of non-lattice data", *The Statistician* 24(3), 179-195 (1975).

The denoise family trains a chain of small machines, each undoing one step of bit-flip noise, and generates new
examples by running the chain from coin flips. The BOLTZLEARN-2 report relates this to the denoising
thermodynamic models of arXiv:2510.23972.

## Leans that aim at a picture

The grid family sets each pixel's lean so that its yes-rate matches a grey level `g`. With no pulls, a thing with
lean `h` is yes with probability `(1 + tanh h) / 2`, so the lean for a target magnetisation `m = 2g - 1` is
`atanh(m)`. With neighbour pulls, the neighbours already contribute input, and the mean-field and TAP corrections
subtract it:

```text
mean field:  h_i = atanh(m_i) - sum_j J_ij m_j
TAP:         h_i = atanh(m_i) - sum_j J_ij m_j + m_i sum_j J_ij^2 (1 - m_j^2)
```

Reading: subtract what the neighbours already push, and (TAP) add back the part of that push which is the
pixel's own influence reflected by its neighbours. `correct: :bethe` solves the pair equations of each edge,
which is exact on a tree; `fit:` refines the leans by measuring the grid itself.

- D. J. Thouless, P. W. Anderson and R. G. Palmer, "Solution of 'Solvable model of a spin glass'",
  *Philosophical Magazine* 35(3), 593-601 (1977).
- H. A. Bethe, "Statistical theory of superlattices", *Proceedings of the Royal Society A* 150, 552-575 (1935).

## Numbers on springs

The numbers family uses real-valued things `x` with energy `U(x) = x.A.x / 2 - b.x`. `drift` runs the overdamped
Langevin update

```text
x <- x - h (A x - b) + sqrt(2 T h) xi,      xi ~ Normal(0, I)
```

Reading: each step moves the numbers downhill on the energy and adds a random kick sized by the temperature. For a
symmetric positive definite `A`, the long-run average of `x` is `A^-1 b`, the solution of `A x = b`, and the
covariance of `x` is `T A^-1 (I - h A / 2)^-1`, which `spread` corrects to `T A^-1`. Sampling a physical system to
solve a linear system is the idea of thermodynamic linear algebra.

- M. Aifer, K. Donatella, M. H. Gordon, S. Duffield, T. Ahle, D. Simpson, G. E. Crooks and P. J. Coles,
  "Thermodynamic linear algebra", arXiv:2308.05660 (2023); *npj Unconventional Computing* (2024).

## Error-correcting codes as springs

The ldpcsettle and ldpcmoves families write a low-density parity-check code as things and pulls: each parity
check becomes helper things and penalty pulls that make an odd check expensive, and each received bit leans its
code bit towards what was received. The calmest arrangement is the most likely codeword. At the Nishimori
temperature (`T = 1` when the leans are the channel's true log-likelihood ratios) the Boltzmann distribution is
the exact posterior over codewords, so averaging each bit there gives the bitwise best decision. That holds only
when the check penalties are infinitely strong. At the finite strengths SETTLE builds, the distribution also
weighs arrangements that are not codewords: on a small code enumerated exactly (20 bits, p 0.05, 400 blocks),
the bitwise average at `T = 1` had 67.5% block error, against 18.0% for the bitwise average over codewords
only (`runs/ldpcmoves/exact.txt`). So the decoders anneal to a calm codeword rather than read that average.

- R. G. Gallager, "Low-density parity-check codes", *IRE Transactions on Information Theory* 8(1), 21-28 (1962).
- D. J. C. MacKay and R. M. Neal, "Near Shannon limit performance of low density parity check codes",
  *Electronics Letters* 32(18), 1645-1646 (1996).
- H. Nishimori, *Statistical Physics of Spin Glasses and Information Processing: An Introduction* (Oxford
  University Press, 2001).

## Further reading

- The campaign ledger on the SETTLE site (`#/results/ledger`) lists every experiment, its sealed predictions and
  its results.
- The experiments' reports on the SETTLE site's results page (`#/results`) give the measurements behind a family,
  with the equations they use and their readings.
