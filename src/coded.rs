//! CODED: text that is compressed, then error-coded, then masked, then stored in a memory (the Hopfield
//! `memory` or the Kanerva `sdm`), and read back by the reverse pipeline, which refuses rather than guesses.
//!
//! ```text
//! model :mind do
//!   memory :m, size: 512
//!   m.remember :cat
//!   m.save_coded :note, "meet at the harbour at nine", compress: :ac, code: :hamming74
//! end
//! run :mind do
//!   m.recall_coded read-address: :note, address-noise: 0.3             # the read-address knows only the note's name
//!   m.recall_coded read-address: :note, address-noise: 0.3, knows: :all # the memory.rs convention: read-address = the stored pattern
//!   m.recall_coded read-address: :ghost                         # never saved: refused, nothing printed
//! end
//! ```
//!
//! The pipeline, both ways:
//!
//! ```text
//!   text -> compress -> [len 8][crc16 16][payload][zeros] = K info bits -> code -> n bits -> x mask -> store
//!   shake from a read-address -> x mask (both signs) -> decode -> unframe -> decompress -> check crc -> text or refuse
//! ```
//!
//! - compress: `:none` (8 bits per byte), `:ac` (arithmetic coding under a static order-1 English byte model
//!   built from `data/coded_train_austen.txt`, shipped with the codec like a dictionary, never stored),
//!   `:lz` (LZSS, window 255, lengths 3..18, literals by a static order-0 Huffman code: a small DEFLATE).
//! - code: `:none`, `:rep3` (each bit three times, majority), `:hamming74` (Hamming 7,4, syndrome decoding),
//!   `:ldpc` with `rate:` (column weight 3, random rows, systematic by Gaussian elimination, decoded by
//!   belief propagation, sum-product in log-likelihood form, at most 50 rounds).
//! - the frame is always padded with zeros to fill the code's K info bits, so the codeword fills the memory.
//! - check: CRC-16/CCITT-FALSE over the text. The decoder tries the recalled state and its mirror image and
//!   accepts only a frame whose check passes; everything else is a refusal with a reason.
//! - the read-address: `knows: :name` (default) is the pipeline applied to an all-zero frame. It is right on every
//!   stored bit that does not depend on the text (the zero padding, and anything a code computes from it)
//!   and a coin on the rest, and it needs no knowledge of the text's length. `knows: :all` is the stored
//!   pattern itself, which is what `m.recall read-address: :note` in memory.rs does.
//!
//! The name is registered in the memory's own list with the tag `#`, so the memory's plain `recall`
//! scoreboard leaves it out, as it does keyed notes. The text is kept in this family's notes (as memory.rs
//! keeps saved text), which the `knows: :all` read-address and the measurements use; decoding never reads it.

use crate::ext::{Claim, Ctx, Ext};
use crate::lex::{err, kw, kwargs, num, only, text, SettleError, Tok};
use crate::memory::{code, seed_of, shake, store_pattern};
use crate::model::{Model, State};
use crate::rng::Rng;
use crate::sdm::View;
use std::sync::OnceLock;

pub struct Coded;

/// The training text of the static English model (public domain; provenance in `data/PROVENANCE.txt`).
pub const TRAIN: &str = include_str!("../data/coded_train_austen.txt");
/// Held-out test text (a different author and book).
pub const TEST: &str = include_str!("../data/coded_test_doyle.txt");

// ---------------------------------------------------------------- bits and the check

/// Bytes to bits, most significant bit first, each bit 0 or 1.
pub fn bytes_to_bits(b: &[u8]) -> Vec<u8> {
    b.iter().flat_map(|&x| (0..8).rev().map(move |k| (x >> k) & 1)).collect()
}

/// Bits to bytes (a trailing partial byte is dropped).
pub fn bits_to_bytes(bits: &[u8]) -> Vec<u8> {
    bits.chunks_exact(8).map(|c| c.iter().fold(0u8, |a, &v| (a << 1) | (v & 1))).collect()
}

fn push_num(out: &mut Vec<u8>, v: u32, width: usize) {
    for k in (0..width).rev() {
        out.push(((v >> k) & 1) as u8);
    }
}

fn read_num(bits: &[u8], at: usize, width: usize) -> u32 {
    (0..width).fold(0u32, |a, k| (a << 1) | *bits.get(at + k).unwrap_or(&0) as u32)
}

/// CRC-16/CCITT-FALSE (polynomial 0x1021, start 0xFFFF). "123456789" gives 0x29B1.
pub fn crc16(data: &[u8]) -> u16 {
    let mut c: u16 = 0xFFFF;
    for &b in data {
        c ^= (b as u16) << 8;
        for _ in 0..8 {
            c = if c & 0x8000 != 0 { (c << 1) ^ 0x1021 } else { c << 1 };
        }
    }
    c
}

// ---------------------------------------------------------------- the static English model

const FREQ_BITS: u32 = 16;
/// Weight of the order-0 distribution when blending it into an order-1 context (fixed before measuring).
const BLEND: f64 = 8.0;

/// The codec's dictionary: order-1 arithmetic-coding tables and an order-0 Huffman code, all built from
/// byte counts of the training text. Nothing of it is stored in a memory.
#[derive(Clone)]
pub struct English {
    /// cum[c * 257 + s]: cumulative frequency of byte s after byte c; cum[c * 257 + 256] is the total.
    cum: Vec<u32>,
    /// Huffman code of each byte: (code, length), most significant bit first.
    huff: Vec<(u32, u8)>,
    /// Canonical decoding: how many codes of each length, and the bytes in canonical order.
    hcount: Vec<u32>,
    hsyms: Vec<u8>,
}

impl English {
    /// The model trained on `TRAIN`, built once.
    pub fn trained() -> &'static English {
        static M: OnceLock<English> = OnceLock::new();
        M.get_or_init(|| English::from_text(TRAIN.as_bytes()))
    }

    pub fn from_text(t: &[u8]) -> English {
        let mut c0 = vec![0.0f64; 256];
        let mut c1 = vec![0.0f64; 256 * 256];
        let mut prev = b' ';
        for &b in t {
            c0[b as usize] += 1.0;
            c1[prev as usize * 256 + b as usize] += 1.0;
            prev = b;
        }
        English::from_counts(&c0, &c1)
    }

    /// A deliberately wrong dictionary for the corrupted-codebook control: the counts of two bytes (`a`
    /// and `b`) are swapped everywhere, in the order-0 counts and in every context.
    pub fn swapped(t: &[u8], a: u8, b: u8) -> English {
        let sw = |x: u8| if x == a { b } else if x == b { a } else { x };
        let mut c0 = vec![0.0f64; 256];
        let mut c1 = vec![0.0f64; 256 * 256];
        let mut prev = b' ';
        for &x in t {
            c0[sw(x) as usize] += 1.0;
            c1[prev as usize * 256 + sw(x) as usize] += 1.0;
            prev = x;
        }
        English::from_counts(&c0, &c1)
    }

    fn from_counts(c0: &[f64], c1: &[f64]) -> English {
        let total0: f64 = c0.iter().sum();
        let p0: Vec<f64> = c0.iter().map(|&c| (c + 0.5) / (total0 + 128.0)).collect();
        let room = (1u32 << FREQ_BITS) - 256;
        let mut cum = vec![0u32; 256 * 257];
        for ctx in 0..256 {
            let row = &c1[ctx * 256..ctx * 256 + 256];
            let tot: f64 = row.iter().sum();
            let mut acc = 0u32;
            for s in 0..256 {
                cum[ctx * 257 + s] = acc;
                let p = (row[s] + BLEND * p0[s]) / (tot + BLEND);
                acc += 1 + (p * room as f64).floor() as u32;
            }
            cum[ctx * 257 + 256] = acc;
        }
        let (huff, hcount, hsyms) = huffman(&p0);
        English { cum, huff, hcount, hsyms }
    }

    /// Ideal code length in bits of `t` under the order-1 model (what arithmetic coding approaches).
    pub fn ideal_bits(&self, t: &[u8]) -> f64 {
        let mut prev = b' ';
        let mut bits = 0.0;
        for &b in t {
            let r = prev as usize * 257;
            let f = (self.cum[r + b as usize + 1] - self.cum[r + b as usize]) as f64;
            bits -= (f / self.cum[r + 256] as f64).log2();
            prev = b;
        }
        bits
    }
}

/// Canonical Huffman code lengths from probabilities (every symbol gets a code).
fn huffman(p: &[f64]) -> (Vec<(u32, u8)>, Vec<u32>, Vec<u8>) {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    let n = p.len();
    // nodes 0..n are leaves; parents get new ids
    let mut parent = vec![usize::MAX; 2 * n];
    let mut heap: BinaryHeap<Reverse<(u64, usize)>> = BinaryHeap::new();
    for (i, &q) in p.iter().enumerate() {
        heap.push(Reverse(((q * 1e9) as u64 + 1, i)));
    }
    let mut next = n;
    while heap.len() > 1 {
        let Reverse((wa, a)) = heap.pop().unwrap();
        let Reverse((wb, b)) = heap.pop().unwrap();
        parent[a] = next;
        parent[b] = next;
        heap.push(Reverse((wa + wb, next)));
        next += 1;
    }
    let mut len = vec![0u8; n];
    for (i, l) in len.iter_mut().enumerate() {
        let mut d = 0;
        let mut k = i;
        while parent[k] != usize::MAX {
            k = parent[k];
            d += 1;
        }
        *l = d;
    }
    let maxlen = *len.iter().max().unwrap() as usize;
    assert!(maxlen <= 31, "Huffman code too long");
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&s| (len[s], s));
    let mut codes = vec![(0u32, 0u8); n];
    let mut hcount = vec![0u32; maxlen + 1];
    let mut c = 0u32;
    let mut prev_len = len[order[0]];
    for (k, &s) in order.iter().enumerate() {
        if k > 0 {
            c += 1;
            c <<= len[s] - prev_len;
        }
        prev_len = len[s];
        codes[s] = (c, len[s]);
        hcount[len[s] as usize] += 1;
    }
    let hsyms = order.iter().map(|&s| s as u8).collect();
    (codes, hcount, hsyms)
}

impl English {
    fn huff_put(&self, out: &mut Vec<u8>, b: u8) {
        let (c, l) = self.huff[b as usize];
        push_num(out, c, l as usize);
    }
    /// Canonical decoding of one byte starting at `bits[*at]`.
    fn huff_get(&self, bits: &[u8], at: &mut usize) -> Option<u8> {
        let (mut code, mut first, mut index) = (0i64, 0i64, 0i64);
        for len in 1..self.hcount.len() {
            code |= *bits.get(*at).unwrap_or(&0) as i64;
            *at += 1;
            let count = self.hcount[len] as i64;
            if code - first < count {
                return Some(self.hsyms[(index + code - first) as usize]);
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        None
    }
}

// ---------------------------------------------------------------- compressors

const TOP: u64 = (1 << 32) - 1;
const HALF: u64 = 1 << 31;
const Q1: u64 = 1 << 30;
const Q3: u64 = 3 << 30;

/// Arithmetic coding (Witten, Neal and Cleary 1987, 32-bit integer form) under the order-1 model.
pub fn ac_encode(t: &[u8], model: &English) -> Vec<u8> {
    let mut out = Vec::new();
    let (mut low, mut high, mut pending) = (0u64, TOP, 0usize);
    let mut emit = |out: &mut Vec<u8>, bit: u8, pending: &mut usize| {
        out.push(bit);
        for _ in 0..*pending {
            out.push(1 - bit);
        }
        *pending = 0;
    };
    let mut prev = b' ';
    for &b in t {
        let r = prev as usize * 257;
        let (lo, hi, tot) = (model.cum[r + b as usize] as u64, model.cum[r + b as usize + 1] as u64, model.cum[r + 256] as u64);
        let range = high - low + 1;
        high = low + range * hi / tot - 1;
        low += range * lo / tot;
        loop {
            if high < HALF {
                emit(&mut out, 0, &mut pending);
            } else if low >= HALF {
                emit(&mut out, 1, &mut pending);
                low -= HALF;
                high -= HALF;
            } else if low >= Q1 && high < Q3 {
                pending += 1;
                low -= Q1;
                high -= Q1;
            } else {
                break;
            }
            low <<= 1;
            high = (high << 1) | 1;
        }
        prev = b;
    }
    pending += 1;
    if low < Q1 {
        emit(&mut out, 0, &mut pending);
    } else {
        emit(&mut out, 1, &mut pending);
    }
    out
}

/// Decode `len` bytes; bits past the end read as 0.
pub fn ac_decode(bits: &[u8], len: usize, model: &English) -> Vec<u8> {
    let bit = |i: usize| *bits.get(i).unwrap_or(&0) as u64;
    let (mut low, mut high) = (0u64, TOP);
    let mut value = (0..32).fold(0u64, |a, i| (a << 1) | bit(i));
    let mut at = 32;
    let mut out = Vec::with_capacity(len);
    let mut prev = b' ';
    for _ in 0..len {
        let r = prev as usize * 257;
        let tot = model.cum[r + 256] as u64;
        let range = high - low + 1;
        let count = ((value - low + 1) * tot - 1) / range;
        // largest s with cum[s] <= count
        let row = &model.cum[r..r + 257];
        let s = row.partition_point(|&c| c as u64 <= count) - 1;
        let s = s.min(255);
        high = low + range * row[s + 1] as u64 / tot - 1;
        low += range * row[s] as u64 / tot;
        loop {
            if high < HALF {
            } else if low >= HALF {
                low -= HALF;
                high -= HALF;
                value -= HALF;
            } else if low >= Q1 && high < Q3 {
                low -= Q1;
                high -= Q1;
                value -= Q1;
            } else {
                break;
            }
            low <<= 1;
            high = (high << 1) | 1;
            value = (value << 1) | bit(at);
            at += 1;
        }
        out.push(s as u8);
        prev = s as u8;
    }
    out
}

const LZ_OFF: usize = 8;
const LZ_LEN: usize = 4;
const LZ_MIN: usize = 3;

/// LZSS: a flag bit, then either a Huffman-coded literal or an 8-bit distance (1..255) and a 4-bit length.
pub fn lz_encode(t: &[u8], model: &English) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    let max_len = LZ_MIN + (1 << LZ_LEN) - 1;
    while i < t.len() {
        let (mut best_len, mut best_off) = (0, 0);
        for off in 1..=((1 << LZ_OFF) - 1).min(i) {
            let mut l = 0;
            while l < max_len && i + l < t.len() && t[i + l] == t[i + l - off] {
                l += 1;
            }
            if l > best_len {
                best_len = l;
                best_off = off;
            }
        }
        if best_len >= LZ_MIN {
            out.push(1);
            push_num(&mut out, best_off as u32, LZ_OFF);
            push_num(&mut out, (best_len - LZ_MIN) as u32, LZ_LEN);
            i += best_len;
        } else {
            out.push(0);
            model.huff_put(&mut out, t[i]);
            i += 1;
        }
    }
    out
}

pub fn lz_decode(bits: &[u8], len: usize, model: &English) -> Option<Vec<u8>> {
    let mut out: Vec<u8> = Vec::with_capacity(len);
    let mut at = 0;
    while out.len() < len {
        if at > bits.len() + 64 {
            return None;
        }
        let flag = *bits.get(at).unwrap_or(&0);
        at += 1;
        if flag == 1 {
            let off = read_num(bits, at, LZ_OFF) as usize;
            let l = read_num(bits, at + LZ_OFF, LZ_LEN) as usize + LZ_MIN;
            at += LZ_OFF + LZ_LEN;
            if off == 0 || off > out.len() {
                return None;
            }
            for _ in 0..l {
                if out.len() == len {
                    break;
                }
                out.push(out[out.len() - off]);
            }
        } else {
            out.push(model.huff_get(bits, &mut at)?);
        }
    }
    Some(out)
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Comp {
    None,
    Ac,
    Lz,
}

impl Comp {
    pub fn parse(s: &str) -> Option<Comp> {
        match s {
            "none" => Some(Comp::None),
            "ac" => Some(Comp::Ac),
            "lz" => Some(Comp::Lz),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Comp::None => "none",
            Comp::Ac => "ac",
            Comp::Lz => "lz",
        }
    }
    pub fn compress(self, t: &[u8], model: &English) -> Vec<u8> {
        match self {
            Comp::None => bytes_to_bits(t),
            Comp::Ac => ac_encode(t, model),
            Comp::Lz => lz_encode(t, model),
        }
    }
    /// `len` bytes from `bits` (bits past the end read as 0). None when the stream cannot be decoded.
    pub fn decompress(self, bits: &[u8], len: usize, model: &English) -> Option<Vec<u8>> {
        match self {
            Comp::None => {
                if bits.len() < 8 * len {
                    return None;
                }
                Some(bits_to_bytes(&bits[..8 * len]))
            }
            Comp::Ac => Some(ac_decode(bits, len, model)),
            Comp::Lz => lz_decode(bits, len, model),
        }
    }
}

// ---------------------------------------------------------------- the frame

/// Length byte plus the 16-bit check.
pub const HEADER_BITS: usize = 24;

/// `[len 8][crc16 16][compressed payload][zeros]`, exactly `k` bits, or why it does not fit.
pub fn frame(t: &[u8], comp: Comp, model: &English, k: usize) -> Result<Vec<u8>, String> {
    if t.len() > 255 {
        return Err(format!("{} bytes is more than the 255 a length byte can name", t.len()));
    }
    let payload = comp.compress(t, model);
    let need = HEADER_BITS + payload.len();
    if need > k {
        return Err(format!("the frame needs {} bits ({} of them the {} payload) and the code carries {}", need, payload.len(), comp.name(), k));
    }
    let mut f = Vec::with_capacity(k);
    push_num(&mut f, t.len() as u32, 8);
    push_num(&mut f, crc16(t) as u32, 16);
    f.extend(payload);
    f.resize(k, 0);
    Ok(f)
}

/// Read a frame back: the text, or the reason it is refused.
pub fn unframe(f: &[u8], comp: Comp, model: &English) -> Result<Vec<u8>, &'static str> {
    let len = read_num(f, 0, 8) as usize;
    let crc = read_num(f, 8, 16) as u16;
    let t = comp.decompress(&f[HEADER_BITS.min(f.len())..], len, model).ok_or("the payload does not decode")?;
    if crc16(&t) != crc {
        return Err("the check failed");
    }
    Ok(t)
}

// ---------------------------------------------------------------- error-correcting codes

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CodeKind {
    None,
    Rep3,
    Hamming74,
    /// Rate in thousandths.
    Ldpc(u32),
}

impl CodeKind {
    pub fn parse(s: &str, rate: f64) -> Option<CodeKind> {
        match s {
            "none" => Some(CodeKind::None),
            "rep3" => Some(CodeKind::Rep3),
            "hamming74" => Some(CodeKind::Hamming74),
            "ldpc" if rate > 0.05 && rate < 0.99 => Some(CodeKind::Ldpc((rate * 1000.0).round() as u32)),
            _ => None,
        }
    }
    pub fn name(self) -> String {
        match self {
            CodeKind::None => "none".into(),
            CodeKind::Rep3 => "rep3".into(),
            CodeKind::Hamming74 => "hamming74".into(),
            CodeKind::Ldpc(r) => format!("ldpc{:.3}", r as f64 / 1000.0),
        }
    }
    /// Inverse of `name`.
    pub fn from_name(s: &str) -> Option<CodeKind> {
        match s.strip_prefix("ldpc") {
            Some(r) => r.parse::<f64>().ok().and_then(|r| CodeKind::parse("ldpc", r)),
            None => CodeKind::parse(s, 0.5),
        }
    }
}

/// A low-density parity-check code: every column of H has three ones in random rows (the lightest of a
/// few random candidates, to keep row weights even). Systematic encoding comes from the reduced row
/// echelon form of H: pivot columns carry parity, the first K free columns carry the information, and any
/// further free columns are fixed at 0 (the decoder knows them).
#[derive(Clone)]
pub struct Ldpc {
    pub n: usize,
    pub rows: Vec<Vec<usize>>,
    pub info: Vec<usize>,
    pub fixed_zero: Vec<usize>,
    pivots: Vec<(usize, Vec<u64>)>,
    var_edges: Vec<Vec<(usize, usize)>>,
}

impl Ldpc {
    pub fn build(n: usize, k: usize, seed: u64) -> Ldpc {
        let m = n - k;
        let mut r = Rng::new(seed);
        let mut weight = vec![0usize; m];
        let mut rows = vec![Vec::new(); m];
        for j in 0..n {
            let mut chosen: Vec<usize> = Vec::new();
            while chosen.len() < 3.min(m) {
                let mut best: Option<usize> = None;
                for _ in 0..4 {
                    let c = r.below(m);
                    if chosen.contains(&c) {
                        continue;
                    }
                    if best.map_or(true, |b| weight[c] < weight[b]) {
                        best = Some(c);
                    }
                }
                if let Some(b) = best {
                    chosen.push(b);
                    weight[b] += 1;
                }
            }
            for &c in &chosen {
                rows[c].push(j);
            }
        }
        let words = (n + 63) / 64;
        let mut dense: Vec<Vec<u64>> = rows
            .iter()
            .map(|row| {
                let mut w = vec![0u64; words];
                for &j in row {
                    w[j / 64] ^= 1 << (j % 64);
                }
                w
            })
            .collect();
        let mut pivot_cols = Vec::new();
        let mut pr = 0;
        for col in 0..n {
            if pr == m {
                break;
            }
            let (wd, bt) = (col / 64, 1u64 << (col % 64));
            let Some(found) = (pr..m).find(|&i| dense[i][wd] & bt != 0) else { continue };
            dense.swap(pr, found);
            for i in 0..m {
                if i != pr && dense[i][wd] & bt != 0 {
                    let src = dense[pr].clone();
                    for (a, b) in dense[i].iter_mut().zip(&src) {
                        *a ^= b;
                    }
                }
            }
            pivot_cols.push(col);
            pr += 1;
        }
        let is_pivot: Vec<bool> = {
            let mut v = vec![false; n];
            for &c in &pivot_cols {
                v[c] = true;
            }
            v
        };
        let free: Vec<usize> = (0..n).filter(|&j| !is_pivot[j]).collect();
        assert!(free.len() >= k, "an LDPC code of length {} cannot carry {} bits", n, k);
        let info = free[..k].to_vec();
        let fixed_zero = free[k..].to_vec();
        let pivots = pivot_cols.iter().enumerate().map(|(i, &c)| (c, dense[i].clone())).collect();
        let mut var_edges = vec![Vec::new(); n];
        for (ri, row) in rows.iter().enumerate() {
            for (e, &j) in row.iter().enumerate() {
                var_edges[j].push((ri, e));
            }
        }
        Ldpc { n, rows, info, fixed_zero, pivots, var_edges }
    }

    pub fn encode(&self, info: &[u8]) -> Vec<u8> {
        let mut c = vec![0u8; self.n];
        for (&p, &b) in self.info.iter().zip(info) {
            c[p] = b;
        }
        for (p, row) in &self.pivots {
            let mut x = 0u8;
            for (wi, &w) in row.iter().enumerate() {
                let mut w = w;
                while w != 0 {
                    let j = wi * 64 + w.trailing_zeros() as usize;
                    w &= w - 1;
                    if j != *p {
                        x ^= c[j];
                    }
                }
            }
            c[*p] = x;
        }
        c
    }

    pub fn syndrome_ok(&self, c: &[u8]) -> bool {
        self.rows.iter().all(|row| row.iter().fold(0u8, |a, &j| a ^ c[j]) == 0)
    }

    /// Belief propagation (sum-product, log-likelihood ratios, positive means 0). Returns the codeword when
    /// every check is satisfied within `rounds`, else None.
    pub fn decode(&self, llr_in: &[f64], rounds: usize) -> Option<Vec<u8>> {
        let mut llr = llr_in.to_vec();
        for &j in &self.fixed_zero {
            llr[j] = 40.0;
        }
        let mut v2c: Vec<Vec<f64>> = self.rows.iter().map(|row| row.iter().map(|&j| llr[j]).collect()).collect();
        let mut c2v: Vec<Vec<f64>> = self.rows.iter().map(|row| vec![0.0; row.len()]).collect();
        let mut bits = vec![0u8; self.n];
        for _ in 0..rounds {
            for (ri, row) in self.rows.iter().enumerate() {
                let t: Vec<f64> = v2c[ri].iter().map(|&m| (m.clamp(-40.0, 40.0) / 2.0).tanh()).collect();
                for e in 0..row.len() {
                    let mut p = 1.0;
                    for (f, &x) in t.iter().enumerate() {
                        if f != e {
                            p *= x;
                        }
                    }
                    c2v[ri][e] = 2.0 * p.clamp(-0.999_999_999_9, 0.999_999_999_9).atanh();
                }
            }
            for j in 0..self.n {
                let total = llr[j] + self.var_edges[j].iter().map(|&(ri, e)| c2v[ri][e]).sum::<f64>();
                bits[j] = (total < 0.0) as u8;
                for &(ri, e) in &self.var_edges[j] {
                    v2c[ri][e] = total - c2v[ri][e];
                }
            }
            if self.syndrome_ok(&bits) {
                return Some(bits);
            }
        }
        None
    }
}

/// Assumed crossover of the memory for hard-decision LDPC input: LLR = ln((1-p)/p) = 3.89 at p = 0.02.
pub const ASSUMED_P: f64 = 0.02;

/// A code sized to a memory of `n` things: K information bits in, at most n coded bits out.
#[derive(Clone)]
pub struct Codec {
    pub kind: CodeKind,
    pub n: usize,
    pub k: usize,
    pub ldpc: Option<Ldpc>,
}

impl Codec {
    /// `codebook_seed` picks the LDPC matrix (1 by default; a different seed is a different codebook).
    pub fn new(kind: CodeKind, n: usize, codebook_seed: u64) -> Codec {
        match kind {
            CodeKind::None => Codec { kind, n, k: n, ldpc: None },
            CodeKind::Rep3 => Codec { kind, n, k: n / 3, ldpc: None },
            CodeKind::Hamming74 => Codec { kind, n, k: 4 * (n / 7), ldpc: None },
            CodeKind::Ldpc(r) => {
                let k = ((n as f64) * r as f64 / 1000.0).floor() as usize;
                let seed = seed_of(&format!("ldpc:{}:{}:{}", n, k, codebook_seed));
                Codec { kind, n, k, ldpc: Some(Ldpc::build(n, k, seed)) }
            }
        }
    }

    pub fn rate(&self) -> f64 {
        self.k as f64 / self.n as f64
    }

    /// Coded bits (length at most n).
    pub fn encode(&self, info: &[u8]) -> Vec<u8> {
        assert_eq!(info.len(), self.k);
        match self.kind {
            CodeKind::None => info.to_vec(),
            CodeKind::Rep3 => info.iter().flat_map(|&b| [b, b, b]).collect(),
            CodeKind::Hamming74 => info
                .chunks(4)
                .flat_map(|d| {
                    let (d1, d2, d3, d4) = (d[0], d[1], d[2], d[3]);
                    // positions 1..7: p1 p2 d1 p3 d2 d3 d4; each parity covers the positions whose index has its bit
                    [d1 ^ d2 ^ d4, d1 ^ d3 ^ d4, d1, d2 ^ d3 ^ d4, d2, d3, d4]
                })
                .collect(),
            CodeKind::Ldpc(_) => self.ldpc.as_ref().unwrap().encode(info),
        }
    }

    /// Information bits from hard bits `y` (length n), or None when the code knows it failed (LDPC only).
    pub fn decode(&self, y: &[u8]) -> Option<Vec<u8>> {
        match self.kind {
            CodeKind::None => Some(y[..self.k].to_vec()),
            CodeKind::Rep3 => Some(y[..3 * self.k].chunks(3).map(|c| ((c[0] + c[1] + c[2]) >= 2) as u8).collect()),
            CodeKind::Hamming74 => Some(
                y[..7 * (self.k / 4)]
                    .chunks(7)
                    .flat_map(|c| {
                        let mut c = [c[0], c[1], c[2], c[3], c[4], c[5], c[6]];
                        let s = (0..7).filter(|&i| c[i] == 1).fold(0usize, |a, i| a ^ (i + 1));
                        if s != 0 {
                            c[s - 1] ^= 1;
                        }
                        [c[2], c[4], c[5], c[6]]
                    })
                    .collect(),
            ),
            CodeKind::Ldpc(_) => {
                let l = self.ldpc.as_ref().unwrap();
                let l0 = ((1.0 - ASSUMED_P) / ASSUMED_P).ln();
                let llr: Vec<f64> = y.iter().map(|&b| if b == 0 { l0 } else { -l0 }).collect();
                l.decode(&llr, 50).map(|c| l.info.iter().map(|&p| c[p]).collect())
            }
        }
    }

    /// Like `decode`, but when LDPC fails, the raw information bits (for partial-recovery measurements).
    pub fn decode_or_raw(&self, y: &[u8]) -> Vec<u8> {
        match (self.decode(y), &self.ldpc) {
            (Some(i), _) => i,
            (None, Some(l)) => l.info.iter().map(|&p| y[p]).collect(),
            (None, None) => unreachable!(),
        }
    }
}

// ---------------------------------------------------------------- the pipeline

/// One saved note's pipeline: compressor, code, and the note's mask (from its name).
#[derive(Clone)]
pub struct Pipeline {
    pub comp: Comp,
    pub codec: Codec,
    pub mask: Vec<f64>,
}

/// The ±1 mask of a coded note's name (distinct from the mask memory.rs gives the same name).
pub fn coded_mask(what: &str, n: usize) -> Vec<f64> {
    code(&format!("coded:{}", what), n)
}

impl Pipeline {
    pub fn new(what: &str, n: usize, comp: Comp, kind: CodeKind, codebook_seed: u64) -> Pipeline {
        Pipeline { comp, codec: Codec::new(kind, n, codebook_seed), mask: coded_mask(what, n) }
    }

    fn to_pattern(&self, cw: &[u8]) -> Vec<f64> {
        (0..self.mask.len()).map(|i| match cw.get(i) {
            Some(&b) => if b == 1 { self.mask[i] } else { -self.mask[i] },
            None => self.mask[i],
        }).collect()
    }

    /// The stored pattern for `t`, or why it does not fit.
    pub fn pattern(&self, t: &[u8], model: &English) -> Result<Vec<f64>, String> {
        let f = frame(t, self.comp, model, self.codec.k)?;
        Ok(self.to_pattern(&self.codec.encode(&f)))
    }

    /// The name-only read-address: the pipeline applied to an all-zero frame.
    pub fn name_read_address(&self) -> Vec<f64> {
        self.to_pattern(&self.codec.encode(&vec![0u8; self.codec.k]))
    }

    fn hard(&self, s: &[f64], sign: f64) -> Vec<u8> {
        s.iter().zip(&self.mask).map(|(&v, &k)| (sign * v * k > 0.0) as u8).collect()
    }

    /// Decode a recalled state: try it and its mirror image; accept the first whose frame passes the check.
    /// Returns the text and the sign used, or the reason for refusing.
    pub fn decode(&self, s: &[f64], model: &English) -> Result<(Vec<u8>, f64), String> {
        let mut why = Vec::new();
        for sign in [1.0, -1.0] {
            let Some(info) = self.codec.decode(&self.hard(s, sign)) else {
                why.push("the code did not converge");
                continue;
            };
            match unframe(&info, self.comp, model) {
                Ok(t) => return Ok((t, sign)),
                Err(e) => why.push(e),
            }
        }
        Err(format!("as stored: {}; as its mirror image: {}", why[0], why[1]))
    }

    /// Best-effort text of a known length, ignoring the check (for partial-recovery measurements only).
    pub fn raw_read(&self, s: &[f64], sign: f64, len: usize, model: &English) -> Vec<u8> {
        let info = self.codec.decode_or_raw(&self.hard(s, sign));
        let t = self.comp.decompress(&info[HEADER_BITS.min(info.len())..], len, model);
        t.unwrap_or_default()
    }
}

// ---------------------------------------------------------------- the statements

enum Store {
    Hop { start: usize, size: usize, fade: f64 },
    Sdm(View),
}

impl Store {
    fn find(m: &Model, name: &str, ln: usize) -> Result<Store, SettleError> {
        if let Some((nums, _)) = m.notes.get(&format!("memory:{}", name)) {
            return Ok(Store::Hop { start: nums[0] as usize, size: nums[1] as usize, fade: nums[2] });
        }
        if m.notes.contains_key(&format!("sdm:{}", name)) {
            return Ok(Store::Sdm(View::load(m, name, ln)?));
        }
        err(ln, format!("no memory or sdm named :{}", name))
    }
    fn size(&self) -> usize {
        match self {
            Store::Hop { size, .. } => *size,
            Store::Sdm(v) => v.n,
        }
    }
    fn notes_key(&self, name: &str) -> String {
        match self {
            Store::Hop { .. } => format!("memory:{}", name),
            Store::Sdm(_) => format!("sdm:{}", name),
        }
    }
}

/// (text, compressor, code) of a saved coded note, from this family's notes.
fn saved(m: &Model, mem: &str, what: &str) -> Option<(String, Comp, CodeKind)> {
    let (_, words) = m.notes.get(&format!("coded:{}", mem))?;
    words.chunks(4).find(|w| w[0] == what).map(|w| (w[1].clone(), Comp::parse(&w[2]).unwrap(), CodeKind::from_name(&w[3]).unwrap()))
}

fn sym_opt(kv: &[(String, Tok)], k: &str, ln: usize) -> Result<Option<String>, SettleError> {
    match kw(kv, k) {
        None => Ok(None),
        Some(Tok::Sym(s)) => Ok(Some(s.clone())),
        Some(_) => err(ln, format!("{}: takes a symbol", k)),
    }
}

fn parse_choice(kv: &[(String, Tok)], ln: usize) -> Result<(Option<Comp>, Option<CodeKind>), SettleError> {
    let comp = match sym_opt(kv, "compress", ln)? {
        None => None,
        Some(s) => Some(Comp::parse(&s).map_or_else(|| err(ln, "compress: takes :none, :ac or :lz"), Ok)?),
    };
    let rate = kw(kv, "rate").map(|v| num(v, ln)).transpose()?.unwrap_or(0.5);
    let code = match sym_opt(kv, "code", ln)? {
        None => None,
        Some(s) => Some(CodeKind::parse(&s, rate).map_or_else(|| err(ln, "code: takes :none, :rep3, :hamming74 or :ldpc (with rate: between 0.05 and 0.99)"), Ok)?),
    };
    Ok((comp, code))
}

fn save(m: &mut Model, mem: &str, what: &str, txt: String, rest: &[Tok], ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let store = Store::find(m, mem, ln)?;
    let kv = kwargs(rest, ln)?;
    only(&kv, &["compress", "code", "rate"], "save_coded", ln)?;
    let (comp, kind) = parse_choice(&kv, ln)?;
    let (comp, kind) = (comp.unwrap_or(Comp::Ac), kind.unwrap_or(CodeKind::Hamming74));
    let key = store.notes_key(mem);
    if m.notes[&key].1.chunks(2).any(|w| w[0] == what) {
        return err(ln, format!(":{} is already stored in :{}", what, mem));
    }
    let n = store.size();
    let pl = Pipeline::new(what, n, comp, kind, 1);
    let model = English::trained();
    let p = pl.pattern(txt.as_bytes(), model).or_else(|e| err(ln, format!("\"{}\" does not fit in :{}: {}", what, mem, e)))?;
    match &store {
        Store::Hop { start, size, fade } => store_pattern(m, *start, *size, *fade, &p),
        Store::Sdm(v) => {
            if v.write(m, &p) == 0 {
                ctx.say(format!("warning: no location of :{} is near :{}, so nothing was written", mem, what));
            }
        }
    }
    let words = &mut m.notes.get_mut(&key).unwrap().1;
    words.push(what.to_string());
    words.push("#".to_string());
    let entry = m.notes.entry(format!("coded:{}", mem)).or_insert((Vec::new(), Vec::new()));
    entry.1.extend([what.to_string(), txt.clone(), comp.name().to_string(), kind.name()]);
    let payload = comp.compress(txt.as_bytes(), model).len();
    ctx.say(format!(
        "save_coded :{} in :{}: {} bytes -> {} payload bits ({}) + {} header -> code {} (rate {:.3}) over {} things",
        what, mem, txt.len(), payload, comp.name(), HEADER_BITS, kind.name(), pl.codec.rate(), n
    ));
    Ok(())
}

fn recall(m: &Model, st: &mut State, mem: &str, rest: &[Tok], ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let store = Store::find(m, mem, ln)?;
    let kv = kwargs(rest, ln)?;
    only(&kv, &["read-address", "address-noise", "knows", "sweeps", "temperature", "iterated-reads", "via", "seed", "compress", "code", "rate", "codebook_seed"], "recall_coded", ln)?;
    if let Some(v) = kw(&kv, "seed") {
        st.rng = Rng::new(num(v, ln)? as u64);
    }
    let what = match kw(&kv, "read-address") {
        Some(Tok::Sym(s)) => s.clone(),
        _ => return err(ln, "recall_coded needs read-address: :name (the name of a coded note)"),
    };
    let damage = kw(&kv, "address-noise").map(|v| num(v, ln)).transpose()?.unwrap_or(0.0);
    let knows = sym_opt(&kv, "knows", ln)?.unwrap_or_else(|| "name".into());
    let (comp_o, kind_o) = parse_choice(&kv, ln)?;
    let cb_seed = kw(&kv, "codebook_seed").map(|v| num(v, ln)).transpose()?.unwrap_or(1.0) as u64;
    let rec = saved(m, mem, &what);
    let (comp, kind) = match &rec {
        Some((_, c, k)) => (comp_o.unwrap_or(*c), kind_o.unwrap_or(*k)),
        None => (comp_o.unwrap_or(Comp::Ac), kind_o.unwrap_or(CodeKind::Hamming74)),
    };
    let n = store.size();
    let model = English::trained();
    let pl = Pipeline::new(&what, n, comp, kind, cb_seed);
    let cue = match knows.as_str() {
        "name" => pl.name_read_address(),
        "all" => match &rec {
            // the stored pattern is rebuilt with the codebook it was saved with
            Some((t, c, k)) => Pipeline::new(&what, n, *c, *k, 1).pattern(t.as_bytes(), model).unwrap(),
            None => return err(ln, format!("knows: :all needs a saved note, and :{} was never saved in :{}", what, mem)),
        },
        _ => return err(ln, "knows: takes :name or :all"),
    };
    let cue: Vec<f64> = cue.iter().map(|&b| if st.rng.unit() < damage { -b } else { b }).collect();
    let (got, how) = match &store {
        Store::Hop { start, .. } => {
            let sweeps = kw(&kv, "sweeps").map(|v| num(v, ln)).transpose()?.unwrap_or(30.0) as usize;
            let temp = kw(&kv, "temperature").map(|v| num(v, ln)).transpose()?.unwrap_or(0.1);
            if temp <= 0.0 {
                return err(ln, "temperature must be above zero");
            }
            (shake(m, st, *start, &cue, sweeps, temp), format!("{} sweeps", sweeps))
        }
        Store::Sdm(v) => {
            let iters = kw(&kv, "iterated-reads").map(|x| num(x, ln)).transpose()?.unwrap_or(10.0) as usize;
            match sym_opt(&kv, "via", ln)?.as_deref() {
                None | Some("addresses") => (v.read_addresses(m, &cue, iters).0, "the address read".to_string()),
                Some("pulls") => (v.read_pulls(m, st, &cue, iters).0, "the pulls read".to_string()),
                Some(_) => return err(ln, "via: takes :addresses or :pulls"),
            }
        }
    };
    let head = format!(
        "recall_coded :{} read-address :{} (knows {}, {:.0}% address-noise) after {}",
        mem,
        what,
        if knows == "all" { "the whole pattern" } else { "only the name" },
        100.0 * damage,
        how
    );
    match pl.decode(&got, model) {
        Ok((t, sign)) => {
            let mirror = if sign < 0.0 { " (from its mirror image)" } else { "" };
            ctx.say(format!("{}: text \"{}\"{}", head, String::from_utf8_lossy(&t), mirror));
        }
        Err(e) => ctx.say(format!("{}: refused ({})", head, e)),
    }
    Ok(())
}

impl Ext for Coded {
    fn name(&self) -> &'static str {
        "coded"
    }

    fn statements(&self) -> &'static [&'static str] {
        &[
            "model/run: m.save_coded :note, \"text\", compress: :ac, code: :hamming74   (m a memory or an sdm; compress :none/:ac/:lz; code :none/:rep3/:hamming74/:ldpc with rate: 0.5)",
            "run: m.recall_coded read-address: :note, address-noise: 0.3, knows: :name, sweeps: 30, temperature: 0.1, seed: 1   (knows :all = read-address from the stored pattern; sdm takes via: and iterated-reads:)",
        ]
    }

    fn model_stmt(&self, m: &mut Model, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim {
        save_stmt(m, t, ln, ctx)
    }

    fn run_stmt(&self, m: &mut Model, st: &mut State, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(name), Tok::Dot, Tok::Ident(v), rest @ ..] if v == "recall_coded" => Some(recall(m, st, name, rest, ln, ctx)),
            _ => save_stmt(m, t, ln, ctx),
        }
    }
}

fn save_stmt(m: &mut Model, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim {
    match t {
        [Tok::Ident(name), Tok::Dot, Tok::Ident(v), Tok::Sym(what), Tok::Comma, s, rest @ ..] if v == "save_coded" => {
            Some(text(s, ln).and_then(|txt| save(m, name, what, txt, rest, ln, ctx)))
        }
        [Tok::Ident(_), Tok::Dot, Tok::Ident(v), ..] if v == "save_coded" => Some(err(ln, "save_coded takes :name, \"text\", then options")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interp::Interp;

    fn run(src: &str) -> Vec<String> {
        Interp::default().exec(src).unwrap_or_else(|e| panic!("{}", e))
    }

    fn passages() -> Vec<&'static [u8]> {
        let t = TEST.as_bytes();
        vec![&t[0..1], &t[0..17], &t[100..161], &t[500..700], &t[2000..2255], b"", b"zzqx\x00\xff\x80 odd bytes \x01"]
    }

    #[test]
    fn crc16_matches_its_standard_check_value() {
        assert_eq!(crc16(b"123456789"), 0x29B1);
    }

    #[test]
    fn every_compressor_round_trips() {
        let m = English::trained();
        for p in passages() {
            for c in [Comp::None, Comp::Ac, Comp::Lz] {
                let bits = c.compress(p, m);
                assert_eq!(c.decompress(&bits, p.len(), m).as_deref(), Some(p), "{:?} on {:?}", c, String::from_utf8_lossy(p));
            }
        }
    }

    #[test]
    fn arithmetic_coding_lands_near_its_ideal_length() {
        let m = English::trained();
        let p = &TEST.as_bytes()[500..700];
        let got = ac_encode(p, m).len() as f64;
        let ideal = m.ideal_bits(p);
        assert!(got >= ideal && got <= ideal + 3.0, "{} bits against the ideal {:.1}", got, ideal);
    }

    #[test]
    fn a_wrong_dictionary_does_not_decode_the_text() {
        // corrupted-codebook control for the compressor: swap the counts of 'e' and 't'
        let m = English::trained();
        let bad = English::swapped(TRAIN.as_bytes(), b'e', b't');
        let p = &TEST.as_bytes()[100..161];
        let bits = ac_encode(p, m);
        assert_ne!(ac_decode(&bits, p.len(), &bad), p);
    }

    #[test]
    fn hamming_corrects_every_single_error_and_rep3_every_single_error_per_block() {
        let c = Codec::new(CodeKind::Hamming74, 70, 1);
        let mut r = Rng::new(3);
        let info: Vec<u8> = (0..c.k).map(|_| (r.unit() < 0.5) as u8).collect();
        let cw = c.encode(&info);
        for i in 0..cw.len() {
            let mut y = cw.clone();
            y[i] ^= 1;
            assert_eq!(c.decode(&y).unwrap(), info, "flip at {}", i);
        }
        let c3 = Codec::new(CodeKind::Rep3, 30, 1);
        let info3: Vec<u8> = (0..c3.k).map(|_| (r.unit() < 0.5) as u8).collect();
        let mut y = c3.encode(&info3);
        for b in 0..c3.k {
            y[3 * b + b % 3] ^= 1;
        }
        assert_eq!(c3.decode(&y).unwrap(), info3);
    }

    #[test]
    fn ldpc_codewords_satisfy_every_check_and_a_few_flips_are_corrected() {
        for rate in [0.5, 0.75] {
            let c = Codec::new(CodeKind::parse("ldpc", rate).unwrap(), 512, 1);
            let l = c.ldpc.as_ref().unwrap();
            assert_eq!(l.info.len(), c.k);
            let mut r = Rng::new(5);
            let info: Vec<u8> = (0..c.k).map(|_| (r.unit() < 0.5) as u8).collect();
            let cw = c.encode(&info);
            assert!(l.syndrome_ok(&cw));
            assert_eq!(c.decode(&cw).unwrap(), info);
            let mut y = cw.clone();
            for _ in 0..4 {
                y[r.below(512)] ^= 1;
            }
            assert_eq!(c.decode(&y).unwrap(), info, "rate {}", rate);
        }
    }

    #[test]
    fn a_frame_round_trips_and_one_flipped_check_bit_is_refused() {
        let m = English::trained();
        let t = &TEST.as_bytes()[0..40];
        for c in [Comp::None, Comp::Ac, Comp::Lz] {
            let mut f = frame(t, c, m, 512).unwrap();
            assert_eq!(unframe(&f, c, m).unwrap(), t);
            f[10] ^= 1;
            assert!(unframe(&f, c, m).is_err());
        }
        assert!(frame(&TEST.as_bytes()[0..70], Comp::None, m, 512).is_err());
    }

    #[test]
    fn the_pipeline_reads_a_pattern_and_its_mirror() {
        let m = English::trained();
        let t = &TEST.as_bytes()[0..30];
        for kind in [CodeKind::None, CodeKind::Rep3, CodeKind::Hamming74, CodeKind::Ldpc(500)] {
            let pl = Pipeline::new("note", 512, Comp::Ac, kind, 1);
            let p = pl.pattern(t, m).unwrap();
            assert_eq!(pl.decode(&p, m).unwrap(), (t.to_vec(), 1.0));
            let q: Vec<f64> = p.iter().map(|x| -x).collect();
            assert_eq!(pl.decode(&q, m).unwrap(), (t.to_vec(), -1.0));
        }
    }

    const SRC: &str = "model :mind do
  memory :m, size: 512
  m.remember :cat
  m.remember :dog
  m.save_coded :note, \"meet at the harbour at nine, bring the red lantern\", compress: :ac, code: :hamming74
  sdm :s, word-size: 512, hard-locations: 2000
  s.save_coded :memo, \"the key is under the third stone\", compress: :ac, code: :ldpc, rate: 0.5
end
";

    #[test]
    fn a_coded_note_comes_back_from_its_name_alone() {
        let out = run(&format!("{}run :mind do\n  m.recall_coded read-address: :note, address-noise: 0.1, seed: 1\n  m.recall_coded read-address: :note, address-noise: 0.3, knows: :all, seed: 2\n  s.recall_coded read-address: :memo, address-noise: 0.05, knows: :all, seed: 3\nend", SRC));
        assert!(out[2].ends_with("text \"meet at the harbour at nine, bring the red lantern\""), "{:?}", out);
        assert!(out[3].ends_with("text \"meet at the harbour at nine, bring the red lantern\""), "{:?}", out);
        assert!(out[4].ends_with("text \"the key is under the third stone\""), "{:?}", out);
    }

    #[test]
    fn a_never_saved_cue_is_refused_and_prints_no_text() {
        let out = run(&format!("{}run :mind do\n  m.recall_coded read-address: :ghost, seed: 1\n  m.recall_coded read-address: :cat, seed: 2\nend", SRC));
        assert!(out[2].contains("refused") && !out[2].contains("text \""), "{}", out[2]);
        assert!(out[3].contains("refused"), "{}", out[3]);
    }

    #[test]
    fn a_wrong_codebook_is_refused() {
        let out = run(&format!(
            "{}run :mind do\n  s.recall_coded read-address: :memo, knows: :all, codebook_seed: 2, seed: 3\n  m.recall_coded read-address: :note, knows: :all, code: :rep3, seed: 1\n  s.recall_coded read-address: :memo, knows: :all, seed: 3\nend",
            SRC
        ));
        assert!(out[2].contains("refused"), "{}", out[2]);
        assert!(out[3].contains("refused"), "{}", out[3]);
        assert!(out[4].contains("text \"the key is under the third stone\""), "vacuity control: {}", out[4]);
    }

    #[test]
    fn a_coded_note_stays_out_of_the_plain_scoreboard_and_too_long_text_is_an_error() {
        let out = run(&format!("{}run :mind do\n  m.recall read-address: :cat, address-noise: 0.1, seed: 1\nend", SRC));
        assert!(out[2].ends_with("-> :cat") && !out[2].contains(":note"), "{}", out[2]);
        let long = "x".repeat(70);
        let e = Interp::default()
            .exec(&format!("model :a do\n  memory :m, size: 512\n  m.save_coded :n, \"{}\", compress: :none, code: :none\nend", long))
            .err()
            .unwrap()
            .0;
        assert!(e.starts_with("line 3:") && e.contains("does not fit"), "{}", e);
    }
}
