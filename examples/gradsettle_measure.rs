//! GRADSETTLE measurement: fit MNIST by Settling (Langevin at temperature 1), beside SGD and Adam at the same
//! budget of gradient rows, and score accuracy, NLL, calibration and the cloud's own doubt.
//!
//! <claudes_code_comments>
//! ** Function List **
//! main()                    - parse args, load MNIST, run every (arm, seed) in parallel over seeds, write outputs
//! Arm                       - one method: sgd, gd (full batch), adam, settle (T = 1), anneal (T 1 -> 0), hot (T = 10)
//! run_arm(...)              - one fit with the descend family's engine; curve rows and final metrics
//! auroc(neg, pos)           - area under the ROC curve by rank sums (ties counted half)
//! entropy(row)              - predictive entropy of one probability row
//!
//! ** Technical Review **
//! - Data: train rows [0, N) of MNIST (pixels / 255, 784 features); eval on the test split (`--split test`) or
//!   on train rows [50,000, 52,000) (`--split val`, for pilots; never overlaps [0, N) for N <= 50,000).
//! - Energy U = sum_i CE_i + |th|^2 / (2 prior^2). Step h = lr / N, so `lr` is the step on the MEAN loss and the
//!   same for every arm; Adam uses `rate`. Budget = steps x batch rows for every arm; gd takes budget / N full
//!   passes. Settle keeps `keep` snapshots over the second half of the run (burn = steps / 2) and predicts by
//!   averaging their probabilities (the Bayesian model average); every arm also reports its final position.
//! - Out of distribution: the eval images with one fixed random permutation of the pixels (seed 99). AUROC of
//!   predictive entropy (and, for clouds, of mutual information) separating real from permuted.
//! - Output: ROW lines (key=value) on stdout; `--out DIR` also writes curve_<tag>.csv per run and summary.json.
//! </claudes_code_comments>

use settle::descend::{descend, metrics, predictive, Data, Method, Opts, Problem, Tick};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;

#[derive(Clone)]
struct Cfg {
    dir: String,
    split: String,
    n: usize,
    model: String,
    hidden: usize,
    prior: f64,
    lr: f64,
    rate: f64,
    lr_to: f64,
    rate_to: f64,
    steps: usize,
    batch: usize,
    keep: usize,
    seeds: Vec<u64>,
    arms: Vec<String>,
    out: Option<String>,
    curve_every: usize,
    shuffle_labels: bool,
    hot: f64,
    temp: f64,
    cold: f64,
}

fn entropy(row: &[f64]) -> f64 {
    row.iter().filter(|&&p| p > 0.0).map(|&p| -p * p.ln()).sum()
}

/// P(score of a positive > score of a negative), ties half.
fn auroc(neg: &[f64], pos: &[f64]) -> f64 {
    let mut all: Vec<(f64, bool)> = neg.iter().map(|&v| (v, false)).chain(pos.iter().map(|&v| (v, true))).collect();
    all.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut rank_sum = 0.0;
    let mut i = 0;
    while i < all.len() {
        let mut j = i;
        while j < all.len() && all[j].0 == all[i].0 {
            j += 1;
        }
        let r = (i + j + 1) as f64 / 2.0;
        for k in i..j {
            if all[k].1 {
                rank_sum += r;
            }
        }
        i = j;
    }
    let (np, nn) = (pos.len() as f64, neg.len() as f64);
    (rank_sum - np * (np + 1.0) / 2.0) / (np * nn)
}

fn arg<T: std::str::FromStr>(args: &[String], k: &str, d: T) -> T {
    args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(d)
}

struct Outcome {
    tag: String,
    fields: BTreeMap<String, f64>,
    curve: Vec<(usize, f64, f64)>,
}

fn run_arm(cfg: &Cfg, arm: &str, seed: u64, train: &Data, eval: &Data, ood: &Data, rot: &Data) -> Outcome {
    let pb = if cfg.model == "net" { Problem::net(cfg.hidden, 10, 1.0, cfg.prior, 0.05) } else { Problem::logistic(10, cfg.prior) };
    let n = train.n;
    let h = cfg.lr / n as f64;
    let mut o = Opts::new(cfg.steps, h, 0.0, seed);
    o.batch = Some(cfg.batch);
    o.burn = cfg.steps / 2;
    o.keep = cfg.keep;
    if cfg.lr_to > 0.0 {
        o.step_to = Some(cfg.lr_to / n as f64);
    }
    match arm {
        "sgd" => {}
        "gd" => {
            o.steps = (cfg.steps * cfg.batch / n).max(20);
            o.batch = None;
            o.burn = o.steps / 2;
        }
        "adam" => {
            o.method = Method::Adam;
            o.step = cfg.rate;
            o.step_to = if cfg.rate_to > 0.0 { Some(cfg.rate_to) } else { None };
        }
        "settle" => o.temp = cfg.temp,
        "anneal" => {
            o.temp = cfg.temp;
            o.cool_to = Some(0.0);
        }
        "hot" => o.temp = cfg.hot,
        "cold" => o.temp = cfg.cold,
        // four walkers from four starts: at T = 0 a deep ensemble, at the cold temperature a cloud of four
        // chains. Four times the budget, said so in rows_seen.
        "sgd4" => {
            o.walkers = 4;
            o.keep = (cfg.keep / 4).max(1);
        }
        "cold4" => {
            o.temp = cfg.cold;
            o.walkers = 4;
            o.keep = (cfg.keep / 4).max(1);
        }
        other => panic!("no arm {}", other),
    }
    // curve: mean batch loss over each window, and the walker's eval accuracy at the window's end
    let mut curve = Vec::new();
    let mut acc_loss = (0.0, 0usize);
    let every = cfg.curve_every.max(1);
    let eval_slice = Data { n: eval.n.min(2000), p: eval.p, x: eval.x[..eval.n.min(2000) * eval.p].to_vec(), y: eval.y[..eval.n.min(2000)].to_vec() };
    let report = if arm == "gd" { 1 } else { 1 };
    let mut cb = |t: &Tick| {
        if t.walker != 0 {
            return;
        }
        acc_loss.0 += t.batch_loss;
        acc_loss.1 += 1;
        let stride = if arm == "gd" { (every * cfg.batch / n).max(1) } else { every };
        if t.step % stride == 0 || t.step == 1 {
            let m = metrics(&pb, &predictive(&pb, &[t.th], &eval_slice), &eval_slice);
            let rows = if arm == "gd" { t.step * n } else { t.step * cfg.batch };
            curve.push((rows, acc_loss.0 / acc_loss.1 as f64, m.accuracy));
            acc_loss = (0.0, 0);
        }
    };
    let r = descend(&pb, train, &o, report, &mut cb);
    let mut f = BTreeMap::new();
    f.insert("secs".into(), r.secs);
    f.insert("rows_seen".into(), r.rows_seen as f64);
    f.insert("blew".into(), if r.blew.is_some() { 1.0 } else { 0.0 });
    let last = &r.last[0];
    let score = |name: &str, samples: &[&[f64]], f: &mut BTreeMap<String, f64>| {
        let pe = predictive(&pb, samples, eval);
        let m = metrics(&pb, &pe, eval);
        f.insert(format!("{}_acc", name), 100.0 * m.accuracy);
        f.insert(format!("{}_nll", name), m.nll);
        f.insert(format!("{}_ece", name), m.ece);
        f.insert(format!("{}_conf", name), m.conf);
        f.insert(format!("{}_mi", name), m.mi);
        let po = predictive(&pb, samples, ood);
        let pr = predictive(&pb, samples, rot);
        let ent_in: Vec<f64> = pe.out.chunks(10).map(entropy).collect();
        let ent_out: Vec<f64> = po.out.chunks(10).map(entropy).collect();
        let ent_rot: Vec<f64> = pr.out.chunks(10).map(entropy).collect();
        f.insert(format!("{}_ood_auroc_entropy", name), auroc(&ent_in, &ent_out));
        f.insert(format!("{}_rot_auroc_entropy", name), auroc(&ent_in, &ent_rot));
        f.insert(format!("{}_ood_mi", name), po.mi.iter().sum::<f64>() / po.mi.len() as f64);
        f.insert(format!("{}_rot_mi", name), pr.mi.iter().sum::<f64>() / pr.mi.len() as f64);
        f.insert(format!("{}_rot_conf", name), metrics(&pb, &pr, rot).conf);
        if samples.len() > 1 {
            f.insert(format!("{}_ood_auroc_mi", name), auroc(&pe.mi, &po.mi));
            f.insert(format!("{}_rot_auroc_mi", name), auroc(&pe.mi, &pr.mi));
            // misclassification detection by doubt
            let k = pe.k;
            let (mut right, mut wrong) = (Vec::new(), Vec::new());
            for r in 0..eval.n {
                let row = &pe.out[r * k..(r + 1) * k];
                let best = (0..k).fold(0, |b, c| if row[c] > row[b] { c } else { b });
                if best == eval.y[r] as usize {
                    right.push(pe.mi[r]);
                } else {
                    wrong.push(pe.mi[r]);
                }
            }
            f.insert(format!("{}_err_auroc_mi", name), auroc(&right, &wrong));
        }
        let (mut right, mut wrong) = (Vec::new(), Vec::new());
        for r in 0..eval.n {
            let row = &pe.out[r * 10..(r + 1) * 10];
            let best = (0..10).fold(0, |b, c| if row[c] > row[b] { c } else { b });
            if best == eval.y[r] as usize {
                right.push(ent_in[r]);
            } else {
                wrong.push(ent_in[r]);
            }
        }
        f.insert(format!("{}_err_auroc_entropy", name), auroc(&right, &wrong));
    };
    score("last", &[last], &mut f);
    if r.last.len() > 1 {
        let lasts: Vec<&[f64]> = r.last.iter().map(|s| s.as_slice()).collect();
        score("ens", &lasts, &mut f);
    }
    if !r.samples.is_empty() && arm != "adam" {
        let refs: Vec<&[f64]> = r.samples.iter().map(|s| s.as_slice()).collect();
        f.insert("cloud_samples".into(), refs.len() as f64);
        score("cloud", &refs, &mut f);
    }
    f.insert("train_loss_last".into(), pb.data_loss(last, train) / n as f64);
    let tag = format!("{}_{}_n{}_{}_s{}", cfg.model, if cfg.shuffle_labels { "shuffled" } else { "real" }, n, arm, seed);
    Outcome { tag, fields: f, curve }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("gradsettle_measure --dir runs/mnist/data --split val|test --n 10000 --model logistic|net [--hidden 100]");
        println!("  --prior 1 --lr 0.1 --lr-to 0.01 --rate 0.001 --rate-to 0.0001 --steps 2000 --batch 100 --keep 100 --seeds 1,2,3");
        println!("  --arms sgd,gd,adam,settle,cold,anneal,hot --temp 1 --cold 0.03 --hot 10 --shuffle-labels --out DIR --curve-every 50");
        return;
    }
    let cfg = Cfg {
        dir: arg(&args, "--dir", "../runs/mnist/data".to_string()),
        split: arg(&args, "--split", "val".to_string()),
        n: arg(&args, "--n", 10_000usize),
        model: arg(&args, "--model", "logistic".to_string()),
        hidden: arg(&args, "--hidden", 100usize),
        prior: arg(&args, "--prior", 1.0f64),
        lr: arg(&args, "--lr", 0.1f64),
        rate: arg(&args, "--rate", 0.001f64),
        lr_to: arg(&args, "--lr-to", 0.0f64),
        rate_to: arg(&args, "--rate-to", 0.0f64),
        steps: arg(&args, "--steps", 2000usize),
        batch: arg(&args, "--batch", 100usize),
        keep: arg(&args, "--keep", 100usize),
        seeds: arg(&args, "--seeds", "1".to_string()).split(',').map(|s| s.parse().unwrap()).collect(),
        arms: arg(&args, "--arms", "sgd,adam,settle".to_string()).split(',').map(String::from).collect(),
        out: args.iter().position(|a| a == "--out").and_then(|i| args.get(i + 1).cloned()),
        curve_every: arg(&args, "--curve-every", 50usize),
        shuffle_labels: args.iter().any(|a| a == "--shuffle-labels"),
        hot: arg(&args, "--hot", 10.0f64),
        temp: arg(&args, "--temp", 1.0f64),
        cold: arg(&args, "--cold", 0.03f64),
    };
    let t0 = std::time::Instant::now();
    let mut train = Data::mnist(&cfg.dir, "train", 0, cfg.n).expect("train split");
    if cfg.shuffle_labels {
        let y: Vec<u8> = train.y.iter().map(|&v| v as u8).collect();
        train.y = settle::mnist::shuffled_labels(&y, 7).iter().map(|&v| v as f64).collect();
    }
    let eval = match cfg.split.as_str() {
        "test" => Data::mnist(&cfg.dir, "test", 0, usize::MAX).expect("test split"),
        "val" => {
            assert!(cfg.n <= 50_000, "val rows start at 50,000");
            Data::mnist(&cfg.dir, "train", 50_000, 2_000).expect("val rows")
        }
        s => panic!("split is val or test, not {}", s),
    };
    let mut ood = eval.clone();
    let mut perm: Vec<usize> = (0..784).collect();
    let mut rng = settle::rng::Rng::new(99);
    for j in (1..784).rev() {
        let r = rng.below(j + 1);
        perm.swap(j, r);
    }
    for r in 0..ood.n {
        let src = eval.row(r).to_vec();
        for i in 0..784 {
            ood.x[r * 784 + i] = src[perm[i]];
        }
    }
    // a harder shift: every eval digit turned a quarter turn clockwise
    let mut rot = eval.clone();
    for r in 0..rot.n {
        let src = eval.row(r).to_vec();
        for yy in 0..28 {
            for xx in 0..28 {
                rot.x[r * 784 + yy * 28 + xx] = src[(27 - xx) * 28 + yy];
            }
        }
    }
    eprintln!("loaded {} train, {} {} rows in {:.1}s", train.n, eval.n, cfg.split, t0.elapsed().as_secs_f64());
    let mut outcomes: Vec<Outcome> = Vec::new();
    for arm in &cfg.arms {
        let res: Vec<Outcome> = std::thread::scope(|sc| {
            let hs: Vec<_> = cfg.seeds.iter().map(|&s| {
                let (cfg, train, eval, ood, rot) = (&cfg, &train, &eval, &ood, &rot);
                sc.spawn(move || run_arm(cfg, arm, s, train, eval, ood, rot))
            }).collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        });
        for o in res {
            let kv: Vec<String> = o.fields.iter().map(|(k, v)| format!("{}={:.6}", k, v)).collect();
            println!("ROW,{},temp={},split={},lr={},lr_to={},rate={},rate_to={},prior={},steps={},batch={},{}", o.tag, cfg.temp, cfg.split, cfg.lr, cfg.lr_to, cfg.rate, cfg.rate_to, cfg.prior, cfg.steps, cfg.batch, kv.join(","));
            std::io::stdout().flush().ok();
            outcomes.push(o);
        }
    }
    if let Some(dir) = &cfg.out {
        fs::create_dir_all(dir).unwrap();
        for o in &outcomes {
            let mut s = String::from("rows_seen,batch_loss,eval_acc_2000\n");
            for (rows, l, a) in &o.curve {
                s.push_str(&format!("{},{:.6},{:.4}\n", rows, l, a));
            }
            fs::write(format!("{}/curve_{}.csv", dir, o.tag), s).unwrap();
        }
        let mut js = String::from("{\n");
        for (i, o) in outcomes.iter().enumerate() {
            let kv: Vec<String> = o.fields.iter().map(|(k, v)| format!("\"{}\": {}", k, if v.is_finite() { format!("{:.6}", v) } else { "null".into() })).collect();
            js.push_str(&format!("  \"{}\": {{{}}}{}\n", o.tag, kv.join(", "), if i + 1 < outcomes.len() { "," } else { "" }));
        }
        js.push_str("}\n");
        let name = format!("{}/summary_{}_{}_n{}{}.json", dir, cfg.model, cfg.split, cfg.n, if cfg.shuffle_labels { "_shuffled" } else { "" });
        fs::write(&name, js).unwrap();
        eprintln!("wrote {}", name);
    }
    eprintln!("total {:.1}s", t0.elapsed().as_secs_f64());
}
