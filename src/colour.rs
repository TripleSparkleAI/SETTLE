//! COLOUR: a colour picture as three grids of p-bits, one per channel (red, green, blue), with no pull between
//! channels. Play a folder of colour frames and score each channel and the whole picture.
//!
//! ```text
//! model :film do
//!   colour :film, width: 160, height: 67, smooth: 0.1     # three grids film_r, film_g, film_b, 10,720 things each
//! end
//! run :film do
//!   play_colour :film, frames: "frames/", out: "out/", sweeps: 20, warm: :yes, read: :soft, correct: :tap
//!   play_colour :film, frames: "frames/", sweeps: 20, against: "other_shot/"    # negative control
//! end
//! ```
//!
//! Each channel is exactly a `grid` (see grid.rs for THE MAPPING, the readouts, `correct:` and `copies:`): a channel
//! value v is a yes-rate, and the lean aims the pixel's yes-rate at v. The three channels settle one after another
//! in the same run and share the run's randomness, but no thing in one channel pulls a thing in another.
//!
//! Scores: per-channel PSNR = 10 log10(1 / MSE_c); overall PSNR = 10 log10(1 / mean_c MSE_c), the usual colour PSNR
//! over all 3 x W x H values. `against:` scores frame k of the output against frame k of another folder instead of
//! the frame whose leans made it (the negative control: it should score low). For the bits readout the line also
//! gives the coin-noise law at no pulls, 10 log10(S / mean g(1 - g)) per channel with S samples per pixel.

use crate::ext::{Claim, Ctx, Ext};
use crate::filmsharp::update_opt;
use crate::grid::{copies_opt, declare, fit_opts, fit_update_opt, invert_opt, invert_word, load, median, play_frame, read_opt, read_pnm, read_word, update_word, Pgm, PlayOpts, Spec};
use crate::lex::{err, kw, kwargs, num, only, text, yes_no, SettleError, Tok, whole};
use crate::model::{Model, State};
use crate::rng::Rng;
use std::path::{Path, PathBuf};

pub struct Colour;

const CHANNELS: [&str; 3] = ["r", "g", "b"];

/// A colour picture: three planes of values in [0, 1], row-major.
#[derive(Clone, Debug, PartialEq)]
pub struct Ppm {
    pub w: usize,
    pub h: usize,
    pub planes: Vec<Vec<f64>>,
}

impl Ppm {
    pub fn channel(&self, c: usize) -> Pgm {
        Pgm { w: self.w, h: self.h, px: self.planes[c].clone() }
    }
}

/// Read a binary (P6) PPM, 8- or 16-bit.
pub fn read_ppm(path: &Path) -> Result<Ppm, String> {
    let (w, h, planes) = read_pnm(path, "P6")?;
    Ok(Ppm { w, h, planes })
}

/// Write an 8-bit binary PPM.
pub fn write_ppm(path: &Path, p: &Ppm) -> Result<(), String> {
    let mut out = format!("P6\n{} {}\n255\n", p.w, p.h).into_bytes();
    for k in 0..p.w * p.h {
        for c in 0..3 {
            out.push((p.planes[c][k].clamp(0.0, 1.0) * 255.0).round() as u8);
        }
    }
    std::fs::write(path, out).map_err(|e| format!("cannot write {}: {}", path.display(), e))
}

fn mse(a: &[f64], b: &[f64]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (x - y) * (x - y)).sum::<f64>() / a.len() as f64
}

fn db(mse: f64) -> f64 {
    if mse <= 1e-12 {
        99.0
    } else {
        10.0 * (1.0 / mse).log10()
    }
}

fn channel_specs(m: &Model, name: &str, ln: usize) -> Result<Vec<Spec>, SettleError> {
    if !m.notes.contains_key(&format!("colour:{}", name)) {
        return err(ln, format!("no colour :{} (declare it with: colour :{}, width: 160, height: 67)", name, name));
    }
    CHANNELS.iter().map(|c| load(m, &format!("{}_{}", name, c), ln)).collect()
}

fn declare_colour(m: &mut Model, name: &str, rest: &[Tok], ln: usize) -> Result<(), SettleError> {
    if m.notes.contains_key(&format!("colour:{}", name)) {
        return err(ln, format!("colour :{} is already declared", name));
    }
    for c in CHANNELS {
        declare(m, &format!("{}_{}", name, c), rest, ln)?;
    }
    m.notes.insert(format!("colour:{}", name), (Vec::new(), Vec::new()));
    Ok(())
}

fn ppm_files(dir: &Path, ln: usize) -> Result<Vec<PathBuf>, SettleError> {
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .or_else(|e| err(ln, format!("cannot read frames folder {}: {}", dir.display(), e)))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|x| x.eq_ignore_ascii_case("ppm")).unwrap_or(false))
        .collect();
    files.sort();
    if files.is_empty() {
        return err(ln, format!("no .ppm frames in {}", dir.display()));
    }
    Ok(files)
}

fn picture(g: &Spec, path: &Path, ln: usize, name: &str) -> Result<Ppm, SettleError> {
    let p = read_ppm(path).or_else(|e| err(ln, e))?;
    if p.w != g.w || p.h != g.h {
        return err(ln, format!("{} is {}x{} but colour :{} is {}x{}", path.display(), p.w, p.h, name, g.w, g.h));
    }
    Ok(p)
}

/// One colour frame's result: the output, the per-channel MSE against the scoring picture, seconds settling.
pub struct ColourOut {
    pub out: Ppm,
    pub mse: [f64; 3],
    pub secs: f64,
}

/// Settle the three channel grids onto one colour picture (each channel with its own warm state in `states`).
pub fn play_colour_frame(m: &mut Model, st: &mut State, specs: &[Spec], pic: &Ppm, score: &Ppm, states: &mut [Vec<f64>], o: &PlayOpts) -> ColourOut {
    let mut planes = Vec::new();
    let (mut msev, mut secs) = ([0.0; 3], 0.0);
    for c in 0..3 {
        let r = play_frame(m, st, &specs[c], &pic.channel(c), &mut states[c], o);
        msev[c] = mse(&r.out, &score.planes[c]);
        secs += r.secs;
        planes.push(r.out);
    }
    ColourOut { out: Ppm { w: pic.w, h: pic.h, planes }, mse: msev, secs }
}

fn play_colour(m: &mut Model, st: &mut State, name: &str, rest: &[Tok], ln: usize, ctx: &mut Ctx) -> Result<(), SettleError> {
    let specs = channel_specs(m, name, ln)?;
    let kv = kwargs(rest, ln)?;
    only(&kv, &["frames", "out", "against", "sweeps", "warm", "read", "keep", "by", "correct", "copies", "update", "fit", "fit_sweeps", "fit_update", "temperature", "seed", "quiet"], "play_colour", ln)?;
    let dir = match kw(&kv, "frames") {
        Some(t) => ctx.path(&text(t, ln)?),
        None => return err(ln, "play_colour needs `frames:` (a folder of .ppm pictures)"),
    };
    let out_dir = kw(&kv, "out").map(|t| text(t, ln)).transpose()?.map(|p| ctx.path(&p));
    let against = kw(&kv, "against").map(|t| text(t, ln)).transpose()?.map(|p| ctx.path(&p));
    let sweeps = match kw(&kv, "sweeps") {
        Some(v) => whole(num(v, ln)?, 0.0, f64::INFINITY, "sweeps:", ln)?,
        None => return err(ln, "play_colour needs `sweeps:`"),
    };
    if sweeps == 0 {
        return err(ln, "sweeps must be at least 1");
    }
    let flag = |k: &str, d: bool| -> Result<bool, SettleError> { Ok(kw(&kv, k).map(|v| yes_no(v, ln)).transpose()?.map(|x| x > 0.0).unwrap_or(d)) };
    let (soft, rb) = read_opt(&kv, ln)?;
    let (fit, fit_sweeps) = fit_opts(&kv, ln)?;
    let keep = kw(&kv, "keep").map(|v| num(v, ln)).transpose()?.unwrap_or(1.0);
    if !(keep > 0.0 && keep <= 1.0) {
        return err(ln, "keep must be above 0 and at most 1");
    }
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
    let files = ppm_files(&dir, ln)?;
    let score_files = match &against {
        Some(a) => {
            let f = ppm_files(a, ln)?;
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
    let samples = (((sweeps as f64 * keep).round() as usize).clamp(1, sweeps) * o.copies) as f64;
    let mut states = vec![Vec::new(), Vec::new(), Vec::new()];
    let (mut per_c, mut overall, mut law, mut secs) = (vec![Vec::new(); 3], Vec::new(), vec![Vec::new(); 3], 0.0);
    for (k, f) in files.iter().enumerate() {
        let pic = picture(&specs[0], f, ln, name)?;
        let score = match &score_files {
            Some(sf) => picture(&specs[0], &sf[k], ln, name)?,
            None => pic.clone(),
        };
        let r = play_colour_frame(m, st, &specs, &pic, &score, &mut states, &o);
        secs += r.secs;
        for c in 0..3 {
            per_c[c].push(db(r.mse[c]));
            let gg = pic.planes[c].iter().map(|g| g * (1.0 - g)).sum::<f64>() / pic.planes[c].len() as f64;
            law[c].push(db(gg / samples));
        }
        overall.push(db((r.mse[0] + r.mse[1] + r.mse[2]) / 3.0));
        let fname = f.file_name().unwrap().to_string_lossy().into_owned();
        if let Some(od) = &out_dir {
            write_ppm(&od.join(&fname), &r.out).or_else(|e| err(ln, e))?;
        }
        if !quiet {
            ctx.say(format!("  {}  R {:.2} G {:.2} B {:.2} overall {:.2} dB", fname, per_c[0][k], per_c[1][k], per_c[2][k], overall[k]));
        }
    }
    let worst = overall.iter().cloned().fold(f64::INFINITY, f64::min);
    let law_txt = if soft || rb { String::new() } else { format!(" (coin-noise law at no pulls: R {:.2} G {:.2} B {:.2})", median(&law[0]), median(&law[1]), median(&law[2])) };
    let copies_txt = if o.copies > 1 { format!("{} copies x ", o.copies) } else { String::new() };
    ctx.say(format!(
        "play_colour :{}: {} frames, {}{} sweeps, {}, {}{}{}{}: median PSNR R {:.2} G {:.2} B {:.2} overall {:.2} dB (worst overall {:.2}){}, {:.1} frames/s settling",
        name,
        files.len(),
        copies_txt,
        sweeps,
        if o.warm { "warm" } else { "cold" },
        read_word(&o),
        invert_word(o.correct),
        update_word(o.update),
        if against.is_some() { ", scored against another shot" } else { "" },
        median(&per_c[0]),
        median(&per_c[1]),
        median(&per_c[2]),
        median(&overall),
        worst,
        law_txt,
        files.len() as f64 / secs.max(1e-9)
    ));
    Ok(())
}

impl Ext for Colour {
    fn name(&self) -> &'static str {
        "colour"
    }

    fn statements(&self) -> &'static [&'static str] {
        &[
            "model: colour :film, width: 160, height: 67, smooth: 0.1",
            "run: play_colour :film, frames: \"dir/\", out: \"dir2/\", sweeps: 20, warm: :yes, read: :soft, correct: :tap, copies: 1, against: \"other/\", seed: 1, quiet: :no",
        ]
    }

    fn model_stmt(&self, m: &mut Model, t: &[Tok], ln: usize, _ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(k), Tok::Sym(name), rest @ ..] if k == "colour" => {
                let rest = if rest.first() == Some(&Tok::Comma) { &rest[1..] } else { rest };
                Some(declare_colour(m, name, rest, ln))
            }
            _ => None,
        }
    }

    fn run_stmt(&self, m: &mut Model, st: &mut State, t: &[Tok], ln: usize, ctx: &mut Ctx) -> Claim {
        match t {
            [Tok::Ident(k), Tok::Sym(name), rest @ ..] if k == "play_colour" => Some(play_colour(m, st, name, rest, ln, ctx)),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::interp::Interp;

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("settle-colour-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Two different colour layouts, 8-bit values.
    fn picture(which: u8, w: usize, h: usize) -> Ppm {
        let planes = (0..3)
            .map(|c| {
                (0..w * h)
                    .map(|k| {
                        let (x, y) = ((k % w) as f64 / w as f64, (k / w) as f64 / h as f64);
                        let v: f64 = if which == b'a' {
                            [0.2 + 0.6 * x, 0.8 - 0.6 * y, if ((x - 0.5).powi(2) + (y - 0.5).powi(2)).sqrt() < 0.25 { 0.9 } else { 0.15 }][c]
                        } else {
                            [if y > 0.5 { 0.85 } else { 0.1 }, 0.3 + 0.5 * y * x, if x < 0.3 { 0.8 } else { 0.25 }][c]
                        };
                        (v * 255.0).round() / 255.0
                    })
                    .collect()
            })
            .collect();
        Ppm { w, h, planes }
    }

    #[test]
    fn ppm_round_trip_keeps_every_byte_and_refuses_a_pgm() {
        let d = scratch("ppm");
        let p = picture(b'a', 13, 7);
        write_ppm(&d.join("a.ppm"), &p).unwrap();
        assert_eq!(read_ppm(&d.join("a.ppm")).unwrap(), p);
        std::fs::write(d.join("g.pgm"), b"P5\n1 1\n255\n\x07").unwrap();
        assert!(read_ppm(&d.join("g.pgm")).unwrap_err().contains("only binary PPM (P6)"));
    }

    #[test]
    fn no_pull_crosses_between_channels() {
        let mut it = Interp::default();
        it.exec("model :f do\n  colour :f, width: 6, height: 5, smooth: 0.2\nend").unwrap();
        let m = &it.models["f"];
        assert_eq!(m.len(), 90);
        for i in 0..m.len() {
            for &(j, _) in &m.adj[i] {
                assert_eq!(i / 30, j / 30, "{} pulls {}", m.names[i], m.names[j]);
            }
        }
    }

    fn run_film(d: &Path, extra: &str) -> Vec<String> {
        let src = format!(
            "model :film do\n  colour :film, width: 30, height: 20, smooth: 0.1\nend\nrun :film do\n  play_colour :film, frames: \"a/\", out: \"out/\", sweeps: 400, read: :soft, correct: :tap, seed: 3{}\nend",
            extra
        );
        let mut it = Interp::in_dir(d.to_path_buf());
        it.exec(&src).unwrap_or_else(|e| panic!("{}", e))
    }

    fn overall(line: &str) -> f64 {
        let t = line.split("overall ").nth(1).unwrap();
        t.split(' ').next().unwrap().parse().unwrap()
    }

    #[test]
    fn colour_frames_are_reproduced_and_another_shot_is_not() {
        // positive run and its negative control: the same output scored against a picture it was never given
        let d = scratch("film");
        for sub in ["a", "b"] {
            std::fs::create_dir_all(d.join(sub)).unwrap();
        }
        for k in 0..2 {
            write_ppm(&d.join("a").join(format!("f{}.ppm", k)), &picture(b'a', 30, 20)).unwrap();
            write_ppm(&d.join("b").join(format!("f{}.ppm", k)), &picture(b'b', 30, 20)).unwrap();
        }
        let own = run_film(&d, "");
        let other = run_film(&d, ", against: \"b/\"");
        let (po, pb) = (overall(own.last().unwrap()), overall(other.last().unwrap()));
        assert!(own.last().unwrap().starts_with("play_colour :film: 2 frames, 400 sweeps, warm, soft: median PSNR R"), "{:?}", own);
        assert!(po > 30.0 && pb < 12.0 && po > pb + 18.0, "own {:.2} other {:.2}", po, pb);
        let out = read_ppm(&d.join("out").join("f1.ppm")).unwrap();
        assert_eq!((out.w, out.h), (30, 20));
    }

    #[test]
    fn bits_readout_without_pulls_sits_on_the_coin_noise_law_per_channel() {
        // exact answer: with no pulls each channel is S independent coins per pixel, PSNR = 10 log10(S / mean g(1-g)).
        // The coins are independent only under Gibbs, so the test names `update: :gibbs` (the default until
        // 2026-10-06); the default rule's draws are anti-correlated and beat the law (the next test).
        let d = scratch("law");
        std::fs::create_dir_all(d.join("a")).unwrap();
        for k in 0..9 {
            write_ppm(&d.join("a").join(format!("f{}.ppm", k)), &picture(b'a', 40, 30)).unwrap();
        }
        let src = "model :film do\n  colour :film, width: 40, height: 30\nend\nrun :film do\n  play_colour :film, frames: \"a/\", sweeps: 30, seed: 5, update: :gibbs\nend";
        let mut it = Interp::in_dir(d.clone());
        let out = it.exec(src).unwrap();
        let line = out.last().unwrap();
        let grab = |tag: &str, after: &str| -> f64 { line.split(after).nth(1).unwrap().split(tag).nth(1).unwrap().trim().chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect::<String>().parse().unwrap() };
        for c in ["R", "G", "B"] {
            let (got, law) = (grab(&format!("{} ", c), "median PSNR"), grab(&format!("{} ", c), "law at no pulls:"));
            assert!((got - law).abs() < 0.4, "{}: measured {:.2} law {:.2} in {}", c, got, law, line);
        }
    }

    #[test]
    fn the_default_rule_beats_the_coin_noise_law_without_pulls() {
        // checkerboard Metropolised Gibbs, the default since 2026-10-06 (lane NEWDEFAULTS): at no pulls a pixel's
        // successive draws are anti-correlated, so its bits read beats S independent coins. Measured on this
        // picture: R 26.54 G 26.80 B 25.40 dB against the law's 21.35, 21.35 and 24.00.
        let d = scratch("lawdefault");
        std::fs::create_dir_all(d.join("a")).unwrap();
        for k in 0..9 {
            write_ppm(&d.join("a").join(format!("f{}.ppm", k)), &picture(b'a', 40, 30)).unwrap();
        }
        let src = "model :film do\n  colour :film, width: 40, height: 30\nend\nrun :film do\n  play_colour :film, frames: \"a/\", sweeps: 30, seed: 5\nend";
        let out = Interp::in_dir(d.clone()).exec(src).unwrap();
        let line = out.last().unwrap();
        let grab = |tag: &str, after: &str| -> f64 { line.split(after).nth(1).unwrap().split(tag).nth(1).unwrap().trim().chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect::<String>().parse().unwrap() };
        for c in ["R", "G", "B"] {
            let (got, law) = (grab(&format!("{} ", c), "median PSNR"), grab(&format!("{} ", c), "law at no pulls:"));
            assert!(got > law + 1.0, "{}: measured {:.2} law {:.2} in {}", c, got, law, line);
        }
    }

    #[test]
    fn against_a_short_folder_is_refused_by_line() {
        let d = scratch("short");
        std::fs::create_dir_all(d.join("a")).unwrap();
        std::fs::create_dir_all(d.join("b")).unwrap();
        for k in 0..2 {
            write_ppm(&d.join("a").join(format!("f{}.ppm", k)), &picture(b'a', 6, 4)).unwrap();
        }
        write_ppm(&d.join("b").join("f0.ppm"), &picture(b'b', 6, 4)).unwrap();
        let mut it = Interp::in_dir(d);
        let e = it.exec("model :f do\n  colour :f, width: 6, height: 4\nend\nrun :f do\n  play_colour :f, frames: \"a/\", sweeps: 2, against: \"b/\"\nend").err().unwrap().0;
        assert!(e.starts_with("line 5:") && e.contains("fewer than the 2 played"), "{}", e);
    }
}
