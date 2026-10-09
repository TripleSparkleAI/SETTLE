//! GRID: one thing per pixel, pulled toward its neighbours, leaning toward a picture. Play a folder of frames.
//!
//! ```text
//! model :horse do
//!   grid :img, width: 150, height: 100, smooth: 0.2   # 15,000 things img_x_y, each pulls its 4 neighbours by 0.2
//!   img.lean_from "frames/horse_00.pgm", by: 1         # set every pixel's lean from a greyscale picture
//! end
//! run :horse do
//!   settle 200
//!   img.show_as "out.pgm"                              # write each pixel's yes-rate as a grey level
//!   play :img, frames: "frames/", out: "out/", sweeps: 10, warm: :yes, read: :bits
//! end
//! ```
//!
//! THE MAPPING. A thing is only ever yes or no, so a grey level cannot be a state. It is a RATE: a pixel of grey g
//! should be yes a fraction g of the time. For a lone thing with lean h at temperature 1 the yes-rate is
//! (1 + tanh h) / 2, so the lean that aims at grey g is atanh(2g - 1). Neighbour pulls add sum_j w m_j to the input,
//! where m_j = 2 g_j - 1 is the neighbour's average; `lean_from` subtracts that (mean-field inversion, the same move as
//! the Python player's bias b = A f). `by:` multiplies the atanh part: 1 aims at the grey, above 1 sharpens it.
//! `correct: :no` skips the subtraction. Greys are clamped to [0.001, 0.999] so the lean stays finite.
//! `correct: :tap` adds the Onsager reaction term of the TAP (second-order) inversion,
//! h_i = atanh(m_i) - sum_j J m_j + m_i sum_j J^2 (1 - m_j^2): a neighbour's pull already contains part of the pixel's
//! own influence echoed back, so plain mean-field subtracts too much; above J = 0.25 on the square grid it flips the
//! lean of a grey region to the wrong sign (the mirror image). `correct: :mean` (same as :yes) is plain mean-field.
//!
//! `copies: K` runs K independent copies of the grid, each for `sweeps:` sweeps, and pools their counts (so the work
//! is K x sweeps pixel updates). Warm copies each continue from their own state on the previous frame.
//!
//! TWO READOUTS in `play`. `read: :bits` is the honest p-bit readout: the share of sweeps each pixel spent at yes.
//! `read: :soft` averages tanh(input) instead, the analogue value a p-bit compares against its noise; it has far less
//! noise, and with `smooth: 0` it returns the lean mapping exactly after one sweep, so it measures little there.
//!
//! `warm: :yes` starts each frame from the state the previous frame ended in; `warm: :no` starts from coin flips.
//! There is no burn-in: every sweep counts toward the budget, and `keep:` sets the share of the last sweeps averaged.

use crate::ext::{Claim, Ctx, Ext};
use crate::filmsharp::{precond_leans, update_opt, Sweeper, Update, DEFAULT_UPDATE};
use crate::filmwarm::{from_word, warm_opts, WarmFit};
use crate::lex::{err, kw, kwargs, num, only, text, yes_no, SettleError, Tok, whole};
use crate::model::{Model, State};
use crate::rng::Rng;
use std::path::Path;
use std::time::Instant;

pub struct Grid;

/// Greys are clamped to this distance from black and white so atanh stays finite.
const GREY_EPS: f64 = 0.001;

/// A greyscale picture with values in [0, 1], row-major.
#[derive(Clone, Debug, PartialEq)]
pub struct Pgm {
    pub w: usize,
    pub h: usize,
    pub px: Vec<f64>,
}

/// Read a binary PNM with `magic` P5 (grey, 1 plane) or P6 (colour, 3 planes), 8- or 16-bit, `#` comments allowed.
/// Returns width, height and one plane of values in [0, 1] per channel.
pub fn read_pnm(path: &Path, magic: &str) -> Result<(usize, usize, Vec<Vec<f64>>), String> {
    let kind = if magic == "P5" { "PGM" } else { "PPM" };
    let b = std::fs::read(path).map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
    let mut i = 0;
    let mut fields: Vec<String> = Vec::new();
    while fields.len() < 4 {
        while i < b.len() && (b[i] as char).is_ascii_whitespace() {
            i += 1;
        }
        if i < b.len() && b[i] == b'#' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        let s = i;
        while i < b.len() && !(b[i] as char).is_ascii_whitespace() {
            i += 1;
        }
        if s == i {
            return Err(format!("{}: the {} header is cut short", path.display(), kind));
        }
        fields.push(String::from_utf8_lossy(&b[s..i]).into_owned());
    }
    i += 1; // exactly one whitespace byte separates the header from the pixels
    if fields[0] != magic {
        return Err(format!("{}: only binary {} ({}) is read, this file starts {:?}", path.display(), kind, magic, fields[0]));
    }
    let parse = |t: &str| t.parse::<usize>().map_err(|_| format!("{}: bad header number {:?}", path.display(), t));
    let (w, h, maxv) = (parse(&fields[1])?, parse(&fields[2])?, parse(&fields[3])?);
    if w == 0 || h == 0 || maxv == 0 || maxv > 65535 {
        return Err(format!("{}: bad size {}x{} or maxval {}", path.display(), w, h, maxv));
    }
    let nc = if magic == "P5" { 1 } else { 3 };
    let bpp = if maxv < 256 { 1 } else { 2 };
    let body = &b[i.min(b.len())..];
    if body.len() < w * h * bpp * nc {
        return Err(format!("{}: {} pixel bytes, expected {}", path.display(), body.len(), w * h * bpp * nc));
    }
    let val = |k: usize| -> f64 {
        let v = if bpp == 1 { body[k] as usize } else { ((body[2 * k] as usize) << 8) | body[2 * k + 1] as usize };
        v as f64 / maxv as f64
    };
    let planes = (0..nc).map(|c| (0..w * h).map(|p| val(p * nc + c)).collect()).collect();
    Ok((w, h, planes))
}

/// Read a binary (P5) PGM, 8- or 16-bit, with `#` comments in the header.
pub fn read_pgm(path: &Path) -> Result<Pgm, String> {
    let (w, h, mut planes) = read_pnm(path, "P5")?;
    Ok(Pgm { w, h, px: planes.remove(0) })
}

/// Write an 8-bit binary PGM, rounding each value in [0, 1] to 0..255.
pub fn write_pgm(path: &Path, p: &Pgm) -> Result<(), String> {
    let mut out = format!("P5\n{} {}\n255\n", p.w, p.h).into_bytes();
    out.extend(p.px.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8));
    std::fs::write(path, out).map_err(|e| format!("cannot write {}: {}", path.display(), e))
}

/// Peak signal-to-noise ratio in dB for pictures in [0, 1] (99 when they match exactly, as the Python player does).
pub fn psnr(a: &[f64], b: &[f64]) -> f64 {
    let mse = a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f64>() / a.len() as f64;
    if mse <= 1e-12 {
        99.0
    } else {
        10.0 * (1.0 / mse).log10()
    }
}

pub struct Spec {
    pub start: usize,
    pub w: usize,
    pub h: usize,
}

pub(crate) fn load(m: &Model, name: &str, ln: usize) -> Result<Spec, SettleError> {
    match m.notes.get(&format!("grid:{}", name)) {
        Some((n, _)) => Ok(Spec { start: n[0] as usize, w: n[1] as usize, h: n[2] as usize }),
        None => err(ln, format!("no grid :{} (declare it with: grid :{}, width: 64, height: 48)", name, name)),
    }
}

pub(crate) fn declare(m: &mut Model, name: &str, rest: &[Tok], ln: usize) -> Result<(), SettleError> {
    if m.notes.contains_key(&format!("grid:{}", name)) {
        return err(ln, format!("grid :{} is already declared", name));
    }
    let kv = kwargs(rest, ln)?;
    only(&kv, &["width", "height", "smooth"], "grid", ln)?;
    let get = |k: &str| kw(&kv, k).map(|v| num(v, ln)).transpose();
    let (w, h) = match (get("width")?, get("height")?) {
        (Some(w), Some(h)) => (w as usize, h as usize),
        _ => return err(ln, "grid needs `width:` and `height:`"),
    };
    if w == 0 || h == 0 || w * h > 4_000_000 {
        return err(ln, "a grid needs between 1 and 4,000,000 pixels");
    }
    let smooth = get("smooth")?.unwrap_or(0.0);
    let start = m.len();
    for y in 0..h {
        for x in 0..w {
            m.add(&format!("{}_{}_{}", name, x, y));
        }
    }
    if smooth != 0.0 {
        for y in 0..h {
            for x in 0..w {
                let i = start + y * w + x;
                if x + 1 < w {
                    m.couple(i, i + 1, smooth);
                }
                if y + 1 < h {
                    m.couple(i, i + w, smooth);
                }
            }
        }
    }
    m.notes.insert(format!("grid:{}", name), (vec![start as f64, w as f64, h as f64, smooth], Vec::new()));
    Ok(())
}

/// How the leans undo the neighbours' pull: not at all, plain mean-field, or TAP (mean-field plus the Onsager term).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Invert {
    None,
    Mean,
    Tap,
    /// The Bethe (pair) inversion, see filmsharp.rs.
    Bethe,
}

/// The inversion when a program names none: TAP since 2026-10-06 (lane NEWDEFAULTS; GRIDPLAYER-2 measured it
/// ahead of mean-field on 20 of 20 exact targets at every pull). `correct: :mean` is the old default.
pub const DEFAULT_INVERT: Invert = Invert::Tap;

/// Read `correct:` (:yes or :mean = mean-field, :tap = TAP, :bethe = Bethe, :no = none; DEFAULT_INVERT, TAP, when
/// absent).
pub fn invert_opt(kv: &[(String, Tok)], ln: usize) -> Result<Invert, SettleError> {
    match kw(kv, "correct") {
        None => Ok(DEFAULT_INVERT),
        Some(Tok::Sym(s)) if s == "tap" => Ok(Invert::Tap),
        Some(Tok::Sym(s)) if s == "bethe" => Ok(Invert::Bethe),
        Some(Tok::Sym(s)) if s == "mean" => Ok(Invert::Mean),
        Some(t) => match yes_no(t, ln) {
            Ok(v) if v > 0.0 => Ok(Invert::Mean),
            Ok(_) => Ok(Invert::None),
            Err(_) => err(ln, "correct: takes :yes or :mean (mean-field), :tap (TAP), :bethe (Bethe) or :no"),
        },
    }
}

/// The leans that aim each pixel's yes-rate at `target` (magnetisations m = 2g - 1, clamped), for one grid.
pub fn leans_for(m: &Model, g: &Spec, target: &[f64], by: f64, inv: Invert) -> Vec<f64> {
    if inv == Invert::Bethe {
        return crate::filmsharp::closed_form_leans(m, g, target, by, inv);
    }
    let (a, b) = (g.start, g.start + g.w * g.h);
    (0..g.w * g.h)
        .map(|k| {
            let i = a + k;
            let mi = target[k];
            let mut lean = by * mi.atanh();
            if inv != Invert::None {
                let nb = m.adj[i].iter().filter(|(j, _)| *j >= a && *j < b);
                lean -= nb.clone().map(|&(j, w)| w * target[j - a]).sum::<f64>();
                if inv == Invert::Tap {
                    lean += mi * nb.map(|&(j, w)| w * w * (1.0 - target[j - a] * target[j - a])).sum::<f64>();
                }
            }
            lean
        })
        .collect()
}

/// Magnetisations m = 2g - 1 of a picture, greys clamped so atanh stays finite.
pub fn magnetisations(pic: &Pgm) -> Vec<f64> {
    pic.px.iter().map(|&v| 2.0 * v.clamp(GREY_EPS, 1.0 - GREY_EPS) - 1.0).collect()
}

/// Set each pixel's lean so its yes-rate aims at the picture's grey (see THE MAPPING above).
pub fn set_leans(m: &mut Model, g: &Spec, pic: &Pgm, by: f64, inv: Invert) {
    let leans = leans_for(m, g, &magnetisations(pic), by, inv);
    m.h[g.start..g.start + g.w * g.h].copy_from_slice(&leans);
}

pub(crate) fn picture_for(g: &Spec, path: &Path, ln: usize, name: &str) -> Result<Pgm, SettleError> {
    let pic = read_pgm(path).or_else(|e| err(ln, e))?;
    if pic.w != g.w || pic.h != g.h {
        return err(ln, format!("{} is {}x{} but grid :{} is {}x{}", path.display(), pic.w, pic.h, name, g.w, g.h));
    }
    Ok(pic)
}

fn lean_from(m: &mut Model, name: &str, path: &str, rest: &[Tok], ln: usize, ctx: &Ctx) -> Result<(), SettleError> {
    let g = load(m, name, ln)?;
    let kv = kwargs(rest, ln)?;
    only(&kv, &["by", "correct"], "lean_from", ln)?;
    let by = kw(&kv, "by").map(|v| num(v, ln)).transpose()?.unwrap_or(1.0);
    let inv = invert_opt(&kv, ln)?;
    let pic = picture_for(&g, &ctx.path(path), ln, name)?;
    set_leans(m, &g, &pic, by, inv);
    m.notes.insert(format!("gridtarget:{}", name), (pic.px, Vec::new()));
    Ok(())
}

fn show_as(m: &Model, st: &State, name: &str, path: &str, rest: &[Tok], ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let g = load(m, name, ln)?;
    let kv = kwargs(rest, ln)?;
    only(&kv, &["from"], "show_as", ln)?;
    let from = match kw(&kv, "from") {
        None => "rate".to_string(),
        Some(Tok::Sym(s)) if s == "rate" || s == "last" => s.clone(),
        Some(_) => return err(ln, "show_as takes from: :rate (yes-rate of the last settle or play) or from: :last"),
    };
    let n = g.w * g.h;
    let px: Vec<f64> = if from == "rate" {
        if st.n == 0 || st.yes.len() < g.start + n {
            return err(ln, "show_as needs a settle or a play first (or use from: :last)");
        }
        (0..n).map(|k| st.yes[g.start + k] as f64 / st.n as f64).collect()
    } else {
        if st.last.len() < g.start + n {
            return err(ln, "show_as from: :last needs a settle, anneal or play first");
        }
        (0..n).map(|k| (st.last[g.start + k] + 1.0) / 2.0).collect()
    };
    let out = ctx.path(path);
    write_pgm(&out, &Pgm { w: g.w, h: g.h, px: px.clone() }).or_else(|e| err(ln, e))?;
    let vs = match m.notes.get(&format!("gridtarget:{}", name)) {
        Some((t, _)) if t.len() == n => format!(", PSNR {:.2} dB against the leans' picture", psnr(&px, t)),
        _ => String::new(),
    };
    ctx.say(format!("show_as :{} -> {} ({}){}", name, path, from, vs));
    Ok(())
}

/// Options for one `play`.
pub struct PlayOpts {
    pub sweeps: usize,
    pub warm: bool,
    pub soft: bool,
    pub keep: f64,
    pub by: f64,
    pub correct: Invert,
    /// Independent copies pooled per frame (1 = one chain).
    pub copies: usize,
    /// Rao-Blackwellised read: average tanh(input) at the moment each pixel draws (see filmsharp.rs).
    pub rb: bool,
    /// The update rule of each sweep.
    pub update: Update,
    /// Secant-Newton fit of the leans before playing: iterations (0 = the closed-form leans) and sweeps each.
    pub fit: usize,
    pub fit_sweeps: usize,
    /// The update rule of the fit's own chain (defaults to `update`).
    pub fit_update: Update,
}

/// The positive-definite floor of the fit's preconditioner (see filmsharp::tap_precond): the smallest inverse
/// response it assumes, so the largest response it assumes is 1 / FIT_FLOOR = 20.
pub const FIT_FLOOR: f64 = 0.05;

impl Default for PlayOpts {
    fn default() -> Self {
        PlayOpts { sweeps: 10, warm: true, soft: false, keep: 1.0, by: 1.0, correct: DEFAULT_INVERT, copies: 1, rb: false, update: DEFAULT_UPDATE, fit: 0, fit_sweeps: 200, fit_update: DEFAULT_UPDATE }
    }
}

/// One frame's result: the reproduced picture, its PSNR against the target, and the seconds spent settling.
pub struct FrameOut {
    pub out: Vec<f64>,
    pub psnr: f64,
    pub secs: f64,
    /// Seconds spent fitting the leans (not part of `secs`) and the RMS grey residual of each fit iteration.
    pub fit_secs: f64,
    pub fit_res: Vec<f64>,
    /// FILMWARM: whether the fit started warm from the previous frame, the sweeps it spent, and the RMS grey change
    /// of the target from the previous frame (all zero or false without `warm_fit:`).
    pub fit_warm: bool,
    pub fit_spent: usize,
    pub fit_change: f64,
}

/// Settle the grid onto one target picture, continuing from `s` (warm) or from coin flips (cold).
/// With `o.copies` = K, `s` holds K whole-model states one after another and the counts of all K are pooled.
pub fn play_frame(m: &mut Model, st: &mut State, g: &Spec, pic: &Pgm, s: &mut Vec<f64>, o: &PlayOpts) -> FrameOut {
    play_frame_with(m, st, g, pic, s, o, None)
}

/// `play_frame` with an optional FILMWARM warm fit (see filmwarm.rs): the fit starts from the previous frame's leans
/// and chain. The fit's seed is drawn from the run exactly as the cold fit's is, so the play below is unchanged.
pub fn play_frame_with(m: &mut Model, st: &mut State, g: &Spec, pic: &Pgm, s: &mut Vec<f64>, o: &PlayOpts, warm: Option<&mut WarmFit>) -> FrameOut {
    set_leans(m, g, pic, o.by, o.correct);
    let (n, len, k_copies) = (g.w * g.h, m.len(), o.copies.max(1));
    let (mut fit_secs, mut fit_res, mut fit_warm, mut fit_spent, mut fit_change) = (0.0, Vec::new(), false, 0usize, 0.0);
    if let (true, Some(wf)) = (o.fit > 0, warm) {
        let t = Instant::now();
        let seed = st.rng.next_u64();
        let h0 = m.h[g.start..g.start + n].to_vec();
        let rec = wf.fit_frame(m, g, &magnetisations(pic), &pic.px, h0, (o.fit, o.fit_sweeps), o.fit_update, 1.0 / st.temp, seed, FIT_FLOOR);
        fit_res = rec.res;
        fit_warm = rec.warm;
        fit_spent = rec.sweeps;
        fit_change = rec.change;
        fit_secs = t.elapsed().as_secs_f64();
    } else if o.fit > 0 {
        // the fit settles its own chain from its own seed (one draw from the run), so the play below starts
        // exactly as it would with the closed-form leans; its sweeps are reported, not hidden in the budget
        let t = Instant::now();
        let seed = st.rng.next_u64();
        let h0 = m.h[g.start..g.start + n].to_vec();
        fit_res = precond_leans(m, g, &magnetisations(pic), h0, o.fit_update, o.fit, o.fit_sweeps, 1.0 / st.temp, seed, FIT_FLOOR).1;
        fit_secs = t.elapsed().as_secs_f64();
        fit_spent = o.fit * o.fit_sweeps.max(4);
    }
    let t0 = Instant::now();
    if s.len() != len * k_copies || !o.warm {
        s.clear();
        for _ in 0..k_copies {
            let (fresh, _) = st.start(m);
            s.extend(fresh);
        }
    }
    let mut free: Vec<usize> = (g.start..g.start + n).filter(|i| !st.held.contains_key(i)).collect();
    let beta = 1.0 / st.temp;
    let kept = ((o.sweeps as f64 * o.keep).round() as usize).clamp(1, o.sweeps.max(1));
    let mut acc = vec![0.0; n];
    let mut rbacc = vec![0.0; n];
    let mut yes = vec![0u64; n];
    // the plain path is State::sweep, which follows the run's core rule (Metropolised Gibbs by default since
    // 2026-10-06), so it stands in for the play's `:gibbs` only while the core rule is Gibbs too
    let plain = o.update == Update::Gibbs && !o.rb && st.update == crate::engine::model::Update::Gibbs;
    let mut sweeper = Sweeper::new(o.update, g, &free);
    for c in 0..k_copies {
        let sc = &mut s[c * len..(c + 1) * len];
        for sw in 0..o.sweeps {
            let counting = sw >= o.sweeps - kept;
            if plain {
                st.sweep(m, sc, &mut free, beta);
            } else {
                let rb = if counting && o.rb { Some(&mut rbacc[..]) } else { None };
                sweeper.sweep(m, g, sc, &mut free, &mut st.rng, beta, rb);
            }
            if counting {
                for k in 0..n {
                    let i = g.start + k;
                    if sc[i] > 0.0 {
                        yes[k] += 1;
                    }
                    if o.soft {
                        acc[k] += (beta * m.input(i, sc)).tanh();
                    }
                }
            }
        }
    }
    let samples = (kept * k_copies) as f64;
    let out: Vec<f64> = if o.rb {
        rbacc.iter().map(|a| (1.0 + a / samples) / 2.0).collect()
    } else if o.soft {
        acc.iter().map(|a| (1.0 + a / samples) / 2.0).collect()
    } else {
        yes.iter().map(|&c| c as f64 / samples).collect()
    };
    let secs = t0.elapsed().as_secs_f64();
    // leave the counts where `show` and `show_as` find them
    st.yes = vec![0; len];
    for k in 0..n {
        st.yes[g.start + k] = yes[k];
    }
    st.n = samples as u64;
    st.last = s[..len].to_vec();
    FrameOut { psnr: psnr(&out, &pic.px), out, secs, fit_secs, fit_res, fit_warm, fit_spent, fit_change }
}

/// Read `read:` as (soft, rb): :bits (yes-rate), :soft (tanh at sweep end), :rb (tanh at each draw).
pub fn read_opt(kv: &[(String, Tok)], ln: usize) -> Result<(bool, bool), SettleError> {
    match kw(kv, "read") {
        None => Ok((false, false)),
        Some(Tok::Sym(s)) if s == "bits" => Ok((false, false)),
        Some(Tok::Sym(s)) if s == "soft" => Ok((true, false)),
        Some(Tok::Sym(s)) if s == "rb" => Ok((false, true)),
        Some(_) => err(ln, "read: takes :bits (yes-rate), :soft (average of tanh of each input) or :rb (Rao-Blackwellised, at each draw)"),
    }
}

/// Read `fit_update:` (the fit chain's rule; the play's `update:` when absent).
pub fn fit_update_opt(kv: &[(String, Tok)], ln: usize) -> Result<Update, SettleError> {
    match kw(kv, "fit_update") {
        None => update_opt(kv, ln),
        Some(t) => update_opt(&[("update".to_string(), t.clone())], ln),
    }
}

/// Read `fit:` (Newton iterations, 0 when absent) and `fit_sweeps:` (sweeps per iteration, 200 when absent).
pub fn fit_opts(kv: &[(String, Tok)], ln: usize) -> Result<(usize, usize), SettleError> {
    let fit = kw(kv, "fit").map(|v| num(v, ln)).transpose()?.unwrap_or(0.0);
    let fs = kw(kv, "fit_sweeps").map(|v| num(v, ln)).transpose()?.unwrap_or(200.0);
    if !(0.0..=1000.0).contains(&fit) || fit.fract() != 0.0 {
        return err(ln, "fit must be a whole number from 0 to 1000");
    }
    if fs < 4.0 || fs.fract() != 0.0 {
        return err(ln, "fit_sweeps must be a whole number of at least 4");
    }
    Ok((fit as usize, fs as usize))
}

pub fn read_word(o: &PlayOpts) -> &'static str {
    if o.rb {
        "rb"
    } else if o.soft {
        "soft"
    } else {
        "bits"
    }
}

pub fn invert_word(i: Invert) -> &'static str {
    match i {
        Invert::Tap => "",
        Invert::Bethe => ", bethe",
        Invert::None => ", uncorrected",
        Invert::Mean => ", mean",
    }
}

pub fn update_word(u: Update) -> &'static str {
    match u {
        Update::Gibbs => ", gibbs",
        Update::Checker => ", checker",
        Update::Metro => ", metro",
        Update::MetroChecker => "",
        Update::Cluster => ", cluster",
    }
}

/// Read `copies:` (a whole number from 1 to 4096; 1 when absent).
pub fn copies_opt(kv: &[(String, Tok)], ln: usize) -> Result<usize, SettleError> {
    match kw(kv, "copies") {
        None => Ok(1),
        Some(v) => {
            let c = num(v, ln)?;
            if !(1.0..=4096.0).contains(&c) || c.fract() != 0.0 {
                return err(ln, "copies must be a whole number from 1 to 4096");
            }
            Ok(c as usize)
        }
    }
}

fn pgm_files(dir: &Path, ln: usize) -> Result<Vec<std::path::PathBuf>, SettleError> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .or_else(|e| err(ln, format!("cannot read frames folder {}: {}", dir.display(), e)))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|x| x.eq_ignore_ascii_case("pgm")).unwrap_or(false))
        .collect();
    files.sort();
    if files.is_empty() {
        return err(ln, format!("no .pgm frames in {}", dir.display()));
    }
    Ok(files)
}

pub(crate) fn median(v: &[f64]) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let k = s.len();
    if k % 2 == 1 {
        s[k / 2]
    } else {
        (s[k / 2 - 1] + s[k / 2]) / 2.0
    }
}

fn play(m: &mut Model, st: &mut State, name: &str, rest: &[Tok], ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let g = load(m, name, ln)?;
    let kv = kwargs(rest, ln)?;
    only(&kv, &["frames", "out", "against", "sweeps", "warm", "read", "keep", "by", "correct", "copies", "update", "fit", "fit_sweeps", "fit_update", "warm_fit", "warm_fit_sweeps", "warm_from", "warm_step", "cut", "temperature", "seed", "quiet"], "play", ln)?;
    let dir = match kw(&kv, "frames") {
        Some(t) => ctx.path(&text(t, ln)?),
        None => return err(ln, "play needs `frames:` (a folder of .pgm pictures)"),
    };
    let out_dir = kw(&kv, "out").map(|t| text(t, ln)).transpose()?.map(|p| ctx.path(&p));
    let against = kw(&kv, "against").map(|t| text(t, ln)).transpose()?.map(|p| ctx.path(&p));
    let sweeps = match kw(&kv, "sweeps") {
        Some(v) => whole(num(v, ln)?, 0.0, f64::INFINITY, "sweeps:", ln)?,
        None => return err(ln, "play needs `sweeps:`"),
    };
    if sweeps == 0 {
        return err(ln, "sweeps must be at least 1");
    }
    let flag = |k: &str, d: bool| -> Result<bool, SettleError> { Ok(kw(&kv, k).map(|v| yes_no(v, ln)).transpose()?.map(|x| x > 0.0).unwrap_or(d)) };
    let (soft, rb) = read_opt(&kv, ln)?;
    let keep = kw(&kv, "keep").map(|v| num(v, ln)).transpose()?.unwrap_or(1.0);
    if !(keep > 0.0 && keep <= 1.0) {
        return err(ln, "keep must be above 0 and at most 1");
    }
    let (fit, fit_sweeps) = fit_opts(&kv, ln)?;
    let mut wf = warm_opts(&kv, fit, fit_sweeps, ln)?;
    let o = PlayOpts {
        sweeps,
        warm: flag("warm", true)?,
        soft,
        keep,
        by: kw(&kv, "by").map(|v| num(v, ln)).transpose()?.unwrap_or(1.0),
        correct: invert_opt(&kv, ln)?,
        copies: copies_opt(&kv, ln)?,
        rb,
        update: update_opt(&kv, ln)?,
        fit,
        fit_sweeps,
        fit_update: fit_update_opt(&kv, ln)?,
    };
    let quiet = flag("quiet", false)?;
    if let Some(v) = kw(&kv, "temperature") {
        st.temp = num(v, ln)?;
        if st.temp <= 0.0 {
            return err(ln, "temperature must be above zero");
        }
    }
    if let Some(v) = kw(&kv, "seed") {
        st.rng = Rng::new(num(v, ln)? as u64);
    }
    let files = pgm_files(&dir, ln)?;
    let score_files = match &against {
        Some(a) => {
            let f = pgm_files(a, ln)?;
            if f.len() < files.len() {
                return err(ln, format!("against: {} has {} frames, fewer than the {} played", a.display(), f.len(), files.len()));
            }
            Some(f)
        }
        None => None,
    };
    if let Some(od) = &out_dir {
        std::fs::create_dir_all(od).or_else(|e| err(ln, format!("cannot make {}: {}", od.display(), e)))?;
    }
    let t_all = Instant::now();
    let mut s = Vec::new();
    let (mut scores, mut settle_secs, mut fit_secs, mut fit_first, mut fit_last) = (Vec::new(), 0.0, 0.0, Vec::new(), Vec::new());
    let (mut spent, mut n_warm) = (Vec::new(), 0usize);
    for f in &files {
        let pic = picture_for(&g, f, ln, name)?;
        let mut r = play_frame_with(m, st, &g, &pic, &mut s, &o, wf.as_mut());
        if let Some(sf) = &score_files {
            r.psnr = psnr(&r.out, &picture_for(&g, &sf[scores.len()], ln, name)?.px);
        }
        let fname = f.file_name().unwrap().to_string_lossy().into_owned();
        if let Some(od) = &out_dir {
            write_pgm(&od.join(&fname), &Pgm { w: g.w, h: g.h, px: r.out }).or_else(|e| err(ln, e))?;
        }
        if !quiet && wf.is_some() {
            ctx.say(format!(
                "  {}  PSNR {:.2} dB  {:.2} ms  fit {} {} sweeps, residual {:.5} -> {:.5}, target change {:.4}",
                fname,
                r.psnr,
                1000.0 * r.secs,
                if r.fit_warm { "warm" } else { "cold" },
                r.fit_spent,
                r.fit_res.first().copied().unwrap_or(0.0),
                r.fit_res.last().copied().unwrap_or(0.0),
                r.fit_change
            ));
        } else if !quiet {
            ctx.say(format!("  {}  PSNR {:.2} dB  {:.2} ms", fname, r.psnr, 1000.0 * r.secs));
        }
        spent.push(r.fit_spent);
        if r.fit_warm {
            n_warm += 1;
        }
        scores.push(r.psnr);
        settle_secs += r.secs;
        fit_secs += r.fit_secs;
        if let (Some(a), Some(b)) = (r.fit_res.first(), r.fit_res.last()) {
            fit_first.push(*a);
            fit_last.push(*b);
        }
    }
    let all = t_all.elapsed().as_secs_f64();
    let worst = scores.iter().cloned().fold(f64::INFINITY, f64::min);
    let fit_txt = if let Some(w) = &wf {
        let later = &scores[1.min(scores.len())..];
        let later_spent = &spent[1.min(spent.len())..];
        format!(
            "; fit {} x {} sweeps{} cold on the first frame, then warm {} x {} from {}{}: {} warm and {} cold frames, fit sweeps {} in all, {:.1} per frame after the first; median PSNR after the first frame {:.2} dB; median grey residual {:.5} -> {:.5}, {:.2} s fitting",
            o.fit,
            o.fit_sweeps,
            update_word(o.fit_update),
            w.iters,
            w.sweeps,
            from_word(w.from),
            if w.cut > 0.0 { format!(", cut above {}", w.cut) } else { String::new() } + &if w.step != 1.0 { format!(", step {}", w.step) } else { String::new() },
            n_warm,
            files.len() - n_warm,
            spent.iter().sum::<usize>(),
            later_spent.iter().sum::<usize>() as f64 / later_spent.len().max(1) as f64,
            median(later),
            median(&fit_first),
            median(&fit_last),
            fit_secs
        )
    } else if o.fit > 0 {
        format!(
            "; fit {} x {} sweeps{}, median grey residual {:.5} -> {:.5}, {:.2} s fitting",
            o.fit,
            o.fit_sweeps,
            update_word(o.fit_update),
            median(&fit_first),
            median(&fit_last),
            fit_secs
        )
    } else {
        String::new()
    };
    ctx.say(format!(
        "play :{}: {} frames, {}{} sweeps, {}, {}{}{}: median PSNR {:.2} dB (worst {:.2}), {:.1} frames/s settling, {:.1} frames/s with file work{}",
        name,
        files.len(),
        if o.copies > 1 { format!("{} copies x ", o.copies) } else { String::new() },
        sweeps,
        if o.warm { "warm" } else { "cold" },
        read_word(&o),
        invert_word(o.correct),
        update_word(o.update),
        median(&scores),
        worst,
        files.len() as f64 / settle_secs.max(1e-9),
        files.len() as f64 / all.max(1e-9),
        fit_txt
    ));
    if against.is_some() {
        ctx.say("  (scored against another folder: the negative control)");
    }
    Ok(())
}

impl Ext for Grid {
    fn name(&self) -> &'static str {
        "grid"
    }

    fn statements(&self) -> &'static [&'static str] {
        &[
            "model: grid :img, width: 64, height: 48, smooth: 0.2",
            "model: img.lean_from \"frame.pgm\", by: 1, correct: :yes",
            "run: img.lean_from \"frame.pgm\", by: 1, correct: :yes",
            "run: img.show_as \"out.pgm\", from: :rate",
            "run: play :img, frames: \"dir/\", out: \"dir2/\", against: \"other/\", sweeps: 10, warm: :yes, read: :bits|:soft|:rb, keep: 1, correct: :tap|:mean|:bethe, copies: 8, update: :metro_checker|:gibbs|:checker|:metro|:cluster, fit: 8, fit_sweeps: 200, fit_update: :cluster, warm_fit: 1, warm_fit_sweeps: 400, warm_from: :correction|:leans, warm_step: 1, cut: 0.25, seed: 1, quiet: :no",
        ]
    }

    fn model_stmt(&self, m: &mut Model, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(k), Tok::Sym(name), rest @ ..] if k == "grid" => {
                let rest = if rest.first() == Some(&Tok::Comma) { &rest[1..] } else { rest };
                Some(declare(m, name, rest, ln))
            }
            [Tok::Ident(name), Tok::Dot, Tok::Ident(v), Tok::Str(p), rest @ ..] if v == "lean_from" => {
                Some(lean_from(m, name, p, rest, ln, ctx))
            }
            _ => None,
        }
    }

    fn run_stmt(&self, m: &mut Model, st: &mut State, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(name), Tok::Dot, Tok::Ident(v), Tok::Str(p), rest @ ..] if v == "lean_from" => {
                Some(lean_from(m, name, p, rest, ln, ctx))
            }
            [Tok::Ident(name), Tok::Dot, Tok::Ident(v), Tok::Str(p), rest @ ..] if v == "show_as" => {
                Some(show_as(m, st, name, p, rest, ln, ctx))
            }
            [Tok::Ident(k), Tok::Sym(name), rest @ ..] if k == "play" => Some(play(m, st, name, rest, ln, ctx)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interp::Interp;
    use std::path::PathBuf;

    /// A fresh scratch folder for one test.
    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("settle-grid-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Picture A: a soft gradient with a dark disc and a bright bar. Picture B: a different layout.
    fn picture(which: u8, w: usize, h: usize) -> Pgm {
        let px = (0..w * h)
            .map(|k| {
                let (x, y) = ((k % w) as f64 / w as f64, (k / w) as f64 / h as f64);
                if which == b'a' {
                    let disc = ((x - 0.35).powi(2) + (y - 0.5).powi(2)).sqrt() < 0.22;
                    let bar = (x - 0.75).abs() < 0.06;
                    if disc { 0.12 } else if bar { 0.9 } else { 0.3 + 0.4 * x }
                } else {
                    let ring = (((x - 0.6).powi(2) + (y - 0.4).powi(2)).sqrt() - 0.25).abs() < 0.07;
                    if ring { 0.95 } else if y > 0.7 { 0.1 } else { 0.8 - 0.4 * y }
                }
            })
            .map(|v: f64| (v * 255.0).round() / 255.0)
            .collect();
        Pgm { w, h, px }
    }

    fn settle_onto(pic: &Pgm, smooth: f64, sweeps: usize, correct: bool, seed: u64) -> Vec<f64> {
        let mut m = Model::default();
        declare(&mut m, "img", &[Tok::Label("width".into()), Tok::Num(pic.w as f64), Tok::Comma, Tok::Label("height".into()), Tok::Num(pic.h as f64), Tok::Comma, Tok::Label("smooth".into()), Tok::Num(smooth)], 1).unwrap();
        let g = load(&m, "img", 1).unwrap();
        let mut st = State::new(seed);
        let mut s = Vec::new();
        let correct = if correct { Invert::Mean } else { Invert::None };
        let o = PlayOpts { sweeps, warm: false, soft: false, keep: 0.9, by: 1.0, correct, copies: 1, ..PlayOpts::default() };
        play_frame(&mut m, &mut st, &g, pic, &mut s, &o).out
    }


    /// PSNR on the second of two frames (B, then B shifted one pixel), warm or cold, soft readout, over `seeds`.
    fn second_frame_psnr(smooth: f64, sweeps: usize, warm: bool, seeds: u64) -> f64 {
        let (w, h) = (40, 30);
        let b = picture(b'b', w, h);
        let shifted = Pgm { w, h, px: (0..w * h).map(|k| b.px[(k / w) * w + ((k % w) + w - 1) % w]).collect() };
        let mut total = 0.0;
        for seed in 0..seeds {
            let mut m = Model::default();
            declare(&mut m, "img", &[Tok::Label("width".into()), Tok::Num(w as f64), Tok::Comma, Tok::Label("height".into()), Tok::Num(h as f64), Tok::Comma, Tok::Label("smooth".into()), Tok::Num(smooth)], 1).unwrap();
            let g = load(&m, "img", 1).unwrap();
            let mut st = State::new(100 + seed);
            let mut s = Vec::new();
            // the measurement below was taken under Gibbs with mean-field leans (the defaults until 2026-10-06), so
            // both are named here; the default rule mixes faster and narrows the warm start's lead
            let first = PlayOpts { sweeps: 50, warm: true, soft: true, keep: 1.0, by: 1.0, correct: Invert::Mean, copies: 1, update: Update::Gibbs, fit_update: Update::Gibbs, ..PlayOpts::default() };
            play_frame(&mut m, &mut st, &g, &b, &mut s, &first);
            let o = PlayOpts { sweeps, warm, soft: true, keep: 1.0, by: 1.0, correct: Invert::Mean, copies: 1, update: Update::Gibbs, fit_update: Update::Gibbs, ..PlayOpts::default() };
            total += play_frame(&mut m, &mut st, &g, &shifted, &mut s, &o).psnr;
        }
        total / seeds as f64
    }

    #[test]
    fn a_warm_start_needs_fewer_sweeps_than_a_cold_one_when_pixels_pull() {
        // measured first (20 seeds, smooth 0.2, soft): warm 2 sweeps 19.84 vs cold 18.52; warm 3 sweeps 21.04 vs cold 19.80
        // and cold needs between 4 (20.74) and 6 (22.21) sweeps to reach warm's 3-sweep PSNR. Bounds below keep margin.
        let (w2, c2) = (second_frame_psnr(0.2, 2, true, 20), second_frame_psnr(0.2, 2, false, 20));
        let (w3, c3) = (second_frame_psnr(0.2, 3, true, 20), second_frame_psnr(0.2, 3, false, 20));
        assert!(w2 > c2 + 0.8, "2 sweeps: warm {:.2} cold {:.2}", w2, c2);
        assert!(c3 < w3 - 0.8, "3 sweeps: warm {:.2} cold {:.2}, so cold needs more than 3 sweeps", w3, c3);
    }

    #[test]
    fn with_no_pulls_the_start_is_forgotten_in_one_sweep() {
        // vacuity control for the warm-start test: with smooth 0 every sweep draws each pixel afresh from its lean,
        // so warm and cold give the same picture (the soft readout is then exact after one sweep)
        let (w, c) = (second_frame_psnr(0.0, 2, true, 5), second_frame_psnr(0.0, 2, false, 5));
        assert!((w - c).abs() < 1e-9 && w > 60.0, "warm {:.2} cold {:.2}", w, c);
    }

    #[test]
    fn pgm_round_trip_keeps_every_byte() {
        let d = scratch("pgm");
        let p = picture(b'a', 37, 23);
        write_pgm(&d.join("a.pgm"), &p).unwrap();
        assert_eq!(read_pgm(&d.join("a.pgm")).unwrap(), p);
        // a header comment and a 16-bit body are read too
        let mut raw = b"P5\n# made by hand\n2 1\n# another\n65535\n".to_vec();
        raw.extend([0xff, 0xff, 0x00, 0x00]);
        std::fs::write(d.join("c.pgm"), raw).unwrap();
        assert_eq!(read_pgm(&d.join("c.pgm")).unwrap().px, vec![1.0, 0.0]);
        std::fs::write(d.join("bad.pgm"), b"P2\n1 1\n255\n7\n").unwrap();
        assert!(read_pgm(&d.join("bad.pgm")).unwrap_err().contains("only binary PGM"));
    }

    #[test]
    fn a_grid_settles_into_the_picture_its_leans_came_from() {
        let a = picture(b'a', 40, 30);
        let got = settle_onto(&a, 0.2, 800, true, 1);
        let p = psnr(&got, &a.px);
        assert!(p > 26.0, "PSNR {:.2}", p);
    }

    #[test]
    fn leans_from_picture_a_do_not_reproduce_picture_b() {
        // negative control: the same settle scored against a picture it was never given
        let (a, b) = (picture(b'a', 40, 30), picture(b'b', 40, 30));
        let got = settle_onto(&a, 0.2, 800, true, 2);
        let (pa, pb) = (psnr(&got, &a.px), psnr(&got, &b.px));
        assert!(pb < 12.0 && pa > pb + 12.0, "against A {:.2}, against B {:.2}", pa, pb);
    }

    #[test]
    fn without_the_mean_field_correction_the_coupled_grid_misses_the_grey() {
        // vacuity control for the correction: same pulls, leans without the neighbour term subtracted
        let a = picture(b'a', 40, 30);
        let with = psnr(&settle_onto(&a, 0.2, 800, true, 3), &a.px);
        let without = psnr(&settle_onto(&a, 0.2, 800, false, 3), &a.px);
        assert!(with > without + 3.0, "with correction {:.2}, without {:.2}", with, without);
    }

    #[test]
    fn play_runs_a_folder_end_to_end_and_writes_every_frame() {
        let d = scratch("play");
        std::fs::create_dir_all(d.join("in")).unwrap();
        for (k, which) in [b'a', b'b', b'a'].iter().enumerate() {
            write_pgm(&d.join("in").join(format!("f{}.pgm", k)), &picture(*which, 24, 16)).unwrap();
        }
        let src = "model :film do
  grid :img, width: 24, height: 16, smooth: 0.2
  img.lean_from \"in/f0.pgm\"
end
run :film do
  settle 300, seed: 1
  img.show_as \"still.pgm\"
  play :img, frames: \"in/\", out: \"out/\", sweeps: 20, warm: :yes, seed: 2
end";
        let mut it = Interp::in_dir(d.clone());
        let out = it.exec(src).unwrap_or_else(|e| panic!("{}", e));
        assert!(out.iter().any(|l| l.starts_with("show_as :img -> still.pgm (rate), PSNR")), "{:?}", out);
        assert!(out.last().unwrap().starts_with("play :img: 3 frames, 20 sweeps, warm, bits: median PSNR"), "{:?}", out);
        for k in 0..3 {
            assert_eq!(read_pgm(&d.join("out").join(format!("f{}.pgm", k))).unwrap().w, 24);
        }
    }

    #[test]
    fn a_picture_of_the_wrong_size_is_refused_by_line() {
        let d = scratch("size");
        write_pgm(&d.join("small.pgm"), &picture(b'a', 8, 8)).unwrap();
        let mut it = Interp::in_dir(d);
        let e = it.exec("model :m do\n  grid :img, width: 9, height: 8\n  img.lean_from \"small.pgm\"\nend").err().unwrap().0;
        assert!(e.starts_with("line 3:") && e.contains("is 8x8 but grid :img is 9x8"), "{}", e);
    }

    /// A w x h grid with pulls `smooth`, declared the way a program does it.
    fn grid_model(w: usize, h: usize, smooth: f64) -> (Model, Spec) {
        let mut m = Model::default();
        declare(&mut m, "img", &[Tok::Label("width".into()), Tok::Num(w as f64), Tok::Comma, Tok::Label("height".into()), Tok::Num(h as f64), Tok::Comma, Tok::Label("smooth".into()), Tok::Num(smooth)], 1).unwrap();
        let g = load(&m, "img", 1).unwrap();
        (m, g)
    }

    /// RMS grey error of the EXACT marginals (by enumeration) of a 4x4 grid whose leans came from `inv`.
    fn exact_grey_error(j: f64, inv: Invert, seed: u64) -> f64 {
        let (mut m, g) = grid_model(4, 4, j);
        let mut r = crate::rng::Rng::new(seed);
        let pic = Pgm { w: 4, h: 4, px: (0..16).map(|_| 0.1 + 0.8 * r.unit()).collect() };
        set_leans(&mut m, &g, &pic, 1.0, inv);
        let got = crate::model::exact_rates(&m, &State::new(0));
        (got.iter().zip(&pic.px).map(|(a, b)| (a - b) * (a - b)).sum::<f64>() / 16.0).sqrt()
    }

    #[test]
    fn tap_leans_land_closer_to_the_exact_marginals_than_mean_field_ones() {
        // the exact answer is enumeration over all 2^16 arrangements; ten targets at pull 0.2
        let (mut mf, mut tap) = (0.0, 0.0);
        for seed in 0..10 {
            mf += exact_grey_error(0.2, Invert::Mean, seed);
            tap += exact_grey_error(0.2, Invert::Tap, seed);
        }
        assert!(tap < mf / 1.5, "mean-field {:.5}, TAP {:.5}", mf / 10.0, tap / 10.0);
    }

    #[test]
    fn with_no_pulls_every_inversion_is_exact() {
        // vacuity control for the TAP test: without pulls there is nothing to correct, so all three agree with the
        // exact marginals to rounding, and the error measured above is the pulls' doing
        for inv in [Invert::None, Invert::Mean, Invert::Tap] {
            let e = exact_grey_error(0.0, inv, 3);
            assert!(e < 1e-12, "{:?}: {}", inv, e);
        }
    }

    #[test]
    fn at_pull_point_three_tap_keeps_the_picture_where_mean_field_flips_it() {
        // above J = 0.25 plain mean-field turns the lean of a grey region against its target (the mirror image).
        // Measured first on this synthetic picture (seed 7): mean-field 14.79 dB, TAP 34.08; bounds keep margin.
        let a = picture(b'a', 40, 30);
        let run = |inv: Invert| {
            let (mut m, g) = grid_model(40, 30, 0.3);
            let mut st = State::new(7);
            let mut s = Vec::new();
            let o = PlayOpts { sweeps: 600, warm: false, soft: true, keep: 0.9, by: 1.0, correct: inv, copies: 1, ..PlayOpts::default() };
            play_frame(&mut m, &mut st, &g, &a, &mut s, &o).psnr
        };
        let (mf, tap) = (run(Invert::Mean), run(Invert::Tap));
        assert!(mf < 18.0 && tap > mf + 12.0, "mean-field {:.2}, TAP {:.2}", mf, tap);
    }

    #[test]
    fn copies_pool_their_counts_and_match_one_long_chain_without_pulls() {
        // with no pulls every sweep is a fresh coin per pixel, so 4 copies x 10 sweeps and 1 copy x 40 are the same
        // estimator; the pooled count is 40 samples either way
        let b = picture(b'b', 40, 30);
        let run = |copies: usize, sweeps: usize, seed: u64| {
            let (mut m, g) = grid_model(40, 30, 0.0);
            let mut st = State::new(seed);
            let mut s = Vec::new();
            let o = PlayOpts { sweeps, warm: false, soft: false, keep: 1.0, by: 1.0, correct: Invert::Mean, copies, ..PlayOpts::default() };
            let p = play_frame(&mut m, &mut st, &g, &b, &mut s, &o).psnr;
            assert_eq!(st.n, 40);
            assert_eq!(s.len(), copies * m.len());
            p
        };
        let (mut one, mut four) = (0.0, 0.0);
        for seed in 0..8 {
            one += run(1, 40, seed);
            four += run(4, 10, 100 + seed);
        }
        assert!((one - four).abs() / 8.0 < 0.3, "1 x 40 {:.3}, 4 x 10 {:.3}", one / 8.0, four / 8.0);
    }

    #[test]
    fn the_rb_read_rides_the_same_chain_as_the_bits_read() {
        // paired by construction: `update: :gibbs` with `read: :rb` draws the same random numbers as the plain
        // sweep, so the yes-counts are identical and only the readout differs
        let a = picture(b'a', 40, 30);
        let run = |rb: bool| {
            let (mut m, g) = grid_model(40, 30, 0.2);
            let mut st = State::new(9);
            let mut s = Vec::new();
            let o = PlayOpts { sweeps: 30, warm: false, rb, correct: Invert::Tap, ..PlayOpts::default() };
            let r = play_frame(&mut m, &mut st, &g, &a, &mut s, &o);
            (st.yes.clone(), r.psnr)
        };
        let ((yb, pb), (yr, pr)) = (run(false), run(true));
        assert_eq!(yb, yr);
        assert!(pr > pb + 6.0, "bits {:.2} rb {:.2}", pb, pr);
    }

    #[test]
    fn with_no_pulls_the_rb_read_is_exact_after_one_sweep() {
        // exact answer: with no neighbours tanh(I) is tanh(lean) = 2g - 1 at every draw
        let a = picture(b'a', 40, 30);
        let (mut m, g) = grid_model(40, 30, 0.0);
        let mut st = State::new(2);
        let mut s = Vec::new();
        let o = PlayOpts { sweeps: 1, warm: false, rb: true, ..PlayOpts::default() };
        let p = play_frame(&mut m, &mut st, &g, &a, &mut s, &o).psnr;
        assert!(p > 60.0, "{}", p);
    }

    #[test]
    fn fitted_leans_hold_the_picture_where_tap_leans_overshoot() {
        // measured first on this synthetic picture (40x30, pull 0.42, cluster chain, soft read 2000 sweeps, seed 4):
        // TAP 10.99 dB, fitted 24.88; the bound keeps margin. Vacuity control: at pull 0 the read returns the leans
        // exactly (99 dB) and the fit leaves them there.
        let a = picture(b'a', 40, 30);
        let run = |j: f64, fit: usize| {
            let (mut m, g) = grid_model(40, 30, j);
            let mut st = State::new(4);
            let mut s = Vec::new();
            let o = PlayOpts { sweeps: 2000, warm: false, soft: true, keep: 0.9, correct: Invert::Tap, fit, fit_sweeps: 400, update: Update::Cluster, fit_update: Update::Cluster, ..PlayOpts::default() };
            play_frame(&mut m, &mut st, &g, &a, &mut s, &o).psnr
        };
        let (tap, fitted) = (run(0.42, 0), run(0.42, 10));
        assert!(tap < 16.0 && fitted > tap + 8.0, "TAP {:.2} fitted {:.2}", tap, fitted);
        let (t0, f0) = (run(0.0, 0), run(0.0, 8));
        assert!(t0 > 60.0 && f0 > 60.0, "no pulls: TAP {:.2} fitted {:.2}", t0, f0);
    }

    #[test]
    fn read_update_and_fit_refuse_bad_values_by_line() {
        for (bad, msg) in [("read: :maybe", "read: takes"), ("update: :fast", "update: takes"), ("fit: 2.5", "fit must be"), ("fit: 2, fit_sweeps: 2", "fit_sweeps must be"), ("fit: 2, fit_update: :slow", "update: takes")] {
            let src = format!("model :m do\n  grid :img, width: 4, height: 4\nend\nrun :m do\n  play :img, frames: \"x/\", sweeps: 2, {}\nend", bad);
            let e = Interp::default().exec(&src).err().unwrap().0;
            assert!(e.starts_with("line 5:") && e.contains(msg), "{}: {}", bad, e);
        }
    }

    #[test]
    fn against_scores_each_frame_against_another_folder() {
        // the negative control statement: the same play scored against pictures it was never given scores low
        let d = scratch("against");
        for sub in ["a", "b"] {
            std::fs::create_dir_all(d.join(sub)).unwrap();
        }
        for k in 0..2 {
            write_pgm(&d.join("a").join(format!("f{}.pgm", k)), &picture(b'a', 24, 16)).unwrap();
            write_pgm(&d.join("b").join(format!("f{}.pgm", k)), &picture(b'b', 24, 16)).unwrap();
        }
        let run = |extra: &str| {
            let src = format!("model :f do\n  grid :img, width: 24, height: 16, smooth: 0.1\nend\nrun :f do\n  play :img, frames: \"a/\", sweeps: 200, read: :soft, correct: :tap, seed: 3{}\nend", extra);
            let mut it = Interp::in_dir(d.clone());
            let out = it.exec(&src).unwrap_or_else(|e| panic!("{}", e));
            let line = out.iter().find(|l| l.starts_with("play :img")).unwrap().clone();
            line.split("median PSNR ").nth(1).unwrap().split(' ').next().unwrap().parse::<f64>().unwrap()
        };
        let (own, other) = (run(""), run(", against: \"b/\""));
        assert!(own > 30.0 && other < 12.0, "own {:.2} other {:.2}", own, other);
    }

    #[test]
    fn correct_and_copies_refuse_bad_values_by_line() {
        let mut it = Interp::default();
        let e = it.exec("model :m do\n  grid :img, width: 4, height: 4\nend\nrun :m do\n  play :img, frames: \"x/\", sweeps: 2, copies: 0\nend").err().unwrap().0;
        assert!(e.starts_with("line 5:") && e.contains("copies must be a whole number"), "{}", e);
        let e = it.exec("model :n do\n  grid :img, width: 4, height: 4\nend\nrun :n do\n  play :img, frames: \"x/\", sweeps: 2, correct: :maybe\nend").err().unwrap().0;
        assert!(e.starts_with("line 5:") && e.contains("correct: takes"), "{}", e);
    }
}
