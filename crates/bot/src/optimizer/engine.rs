use rand::rngs::SmallRng;
use rand::SeedableRng;
use rayon::prelude::*;

use crate::config::{CostsConfig, OptimizerConfig, PaperTradingConfig, TaxConfig};

use super::evaluator::{self, pick_random_asset_interval, CandlePool, EvalResult};
use super::genome::Genome;

/// Ergebnis einer abgeschlossenen Optimierung.
pub struct OptimizationResult<G: Genome> {
    pub winner_a: G,
    pub winner_b: G,
    pub score_a: f64,
    pub score_b: f64,
    pub generations: u32,
}

/// Protokolleintrag einer Generation (enthält frische Scores, nicht akkumulierte).
#[derive(Debug)]
pub struct GenerationLog {
    pub generation: u32,
    pub fresh_a: f64,     // bester Score aus Gruppe A *dieser* Generation
    pub fresh_b: f64,     // bester Score aus Gruppe B *dieser* Generation
    pub asset: String,    // gemeinsames Asset dieser Generation
    pub interval: String, // gemeinsames Intervall dieser Generation
    pub all_time_best: f64,
    pub best_gen: u32,
}

pub fn run<G: Genome>(
    seed_genome: &G,
    prev_winners: Option<(G, G)>,
    pool: &CandlePool,
    opt_cfg: &OptimizerConfig,
    paper_cfg: &PaperTradingConfig,
    costs_cfg: &CostsConfig,
    tax_cfg: &TaxConfig,
) -> (OptimizationResult<G>, Vec<GenerationLog>) {
    let mut rng = SmallRng::from_entropy();
    let pop_size = opt_cfg.population_size.max(4);
    let half = pop_size / 2;

    // ── Generation 0 ─────────────────────────────────────────────────────────
    let population: Vec<G> = match prev_winners {
        Some((prev_a, prev_b)) => {
            println!("  → Vorherige Gewinner als Startpunkt geladen.\n");
            let mut pop = vec![prev_a.clone(), prev_b.clone()];
            let remaining = pop_size.saturating_sub(2);
            for _ in 0..remaining / 2 {
                pop.push(prev_a.mutate(opt_cfg.mutation_magnitude, &mut rng));
            }
            for _ in 0..remaining - remaining / 2 {
                pop.push(prev_b.mutate(opt_cfg.mutation_magnitude, &mut rng));
            }
            pop
        }
        None => (0..pop_size)
            .map(|_| seed_genome.random_like(&mut rng))
            .collect(),
    };

    // Gen 0: gemeinsames Asset+Intervall wählen, ganze Population evaluieren
    let (asset0, iv0) = pick_random_asset_interval(pool, &mut rng).expect("CandlePool ist leer");

    let all_results = eval_parallel(
        &population,
        pool,
        &asset0,
        &iv0,
        opt_cfg,
        paper_cfg,
        costs_cfg,
        tax_cfg,
    );

    let group_a_pop: Vec<G> = population[..half].to_vec();
    let group_a_res: Vec<&EvalResult> = all_results[..half].iter().collect();
    let group_b_pop: Vec<G> = population[half..].to_vec();
    let group_b_res: Vec<&EvalResult> = all_results[half..].iter().collect();

    let (mut winner_a, mut score_a) = best_from_refs(&group_a_pop, &group_a_res);
    let (mut winner_b, mut score_b) = best_from_refs(&group_b_pop, &group_b_res);

    let mut all_time_best = score_a.max(score_b);
    let mut all_time_best_gen = 0u32;
    let mut all_time_best_genome = if score_a >= score_b {
        winner_a.clone()
    } else {
        winner_b.clone()
    };

    let mut logs = Vec::new();
    print_header();
    print_gen(
        0,
        score_a,
        score_b,
        &asset0,
        &iv0,
        all_time_best,
        all_time_best_gen,
        true,
    );
    logs.push(make_log(
        0,
        score_a,
        score_b,
        asset0,
        iv0,
        all_time_best,
        all_time_best_gen,
    ));

    // ── Generationen 1..max ───────────────────────────────────────────────────
    for gen in 1..=opt_cfg.max_generations {
        // Pro Generation ein gemeinsames Asset+Intervall → fairer A-vs-B-Vergleich
        let (asset, iv) = pick_random_asset_interval(pool, &mut rng).expect("CandlePool ist leer");

        let seeds_a: Vec<G> = (0..half)
            .map(|_| winner_a.mutate(opt_cfg.mutation_magnitude, &mut rng))
            .collect();
        let seeds_b: Vec<G> = (0..half)
            .map(|_| winner_b.mutate(opt_cfg.mutation_magnitude, &mut rng))
            .collect();

        let res_a = eval_parallel(
            &seeds_a, pool, &asset, &iv, opt_cfg, paper_cfg, costs_cfg, tax_cfg,
        );
        let res_b = eval_parallel(
            &seeds_b, pool, &asset, &iv, opt_cfg, paper_cfg, costs_cfg, tax_cfg,
        );

        // Frische Scores dieser Generation (was haben die Mutationen *jetzt* geschafft?)
        let (best_a, fresh_a) = best_from_owned(&seeds_a, &res_a);
        let (best_b, fresh_b) = best_from_owned(&seeds_b, &res_b);

        // Elitist: Gewinner nur ersetzen wenn frischer Score besser als akkumulierter Bestwert
        if fresh_a > score_a {
            winner_a = best_a;
            score_a = fresh_a;
        }
        if fresh_b > score_b {
            winner_b = best_b;
            score_b = fresh_b;
        }

        let gen_best = fresh_a.max(fresh_b);
        let is_new = gen_best > all_time_best;
        if is_new {
            all_time_best = gen_best;
            all_time_best_gen = gen;
            all_time_best_genome = if fresh_a >= fresh_b {
                winner_a.clone()
            } else {
                winner_b.clone()
            };
        }

        print_gen(
            gen,
            fresh_a,
            fresh_b,
            &asset,
            &iv,
            all_time_best,
            all_time_best_gen,
            is_new,
        );
        logs.push(make_log(
            gen,
            fresh_a,
            fresh_b,
            asset,
            iv,
            all_time_best,
            all_time_best_gen,
        ));
    }

    // ── Footer ───────────────────────────────────────────────────────────────
    let valid_a: Vec<f64> = logs.iter().map(|l| l.fresh_a).filter(|s| s.is_finite()).collect();
    let valid_b: Vec<f64> = logs.iter().map(|l| l.fresh_b).filter(|s| s.is_finite()).collect();
    let avg_a = if valid_a.is_empty() { f64::NAN } else { valid_a.iter().sum::<f64>() / valid_a.len() as f64 };
    let avg_b = if valid_b.is_empty() { f64::NAN } else { valid_b.iter().sum::<f64>() / valid_b.len() as f64 };

    println!();
    println!("  ─────┴───────────────────┴────────────────────────┴──────────────────────");
    println!("  Avg  │                   │  {:>9}  {:>9}  │  {:>9}  (Gen {:>4})",
             format!("{avg_a:+.4}"), format!("{avg_b:+.4}"),
             format!("{all_time_best:+.4}"), all_time_best_gen);
    println!();

    // Winner A = bestes Individuum insgesamt
    if score_b > score_a {
        std::mem::swap(&mut winner_a, &mut winner_b);
        std::mem::swap(&mut score_a, &mut score_b);
    }
    if all_time_best > score_a {
        winner_a = all_time_best_genome;
        score_a = all_time_best;
    }

    let result = OptimizationResult {
        winner_a,
        winner_b,
        score_a,
        score_b,
        generations: opt_cfg.max_generations,
    };
    (result, logs)
}

// ── Hilfsfunktionen ───────────────────────────────────────────────────────────

fn eval_parallel<G: Genome>(
    population: &[G],
    pool: &CandlePool,
    asset: &str,
    interval: &str,
    opt_cfg: &OptimizerConfig,
    paper_cfg: &PaperTradingConfig,
    costs_cfg: &CostsConfig,
    tax_cfg: &TaxConfig,
) -> Vec<EvalResult> {
    population
        .par_iter()
        .map(|genome| {
            let mut rng = SmallRng::from_entropy();
            evaluator::evaluate(
                genome,
                pool,
                asset,
                interval,
                opt_cfg.min_window_candles,
                paper_cfg,
                costs_cfg,
                tax_cfg,
                &opt_cfg.fitness,
                &mut rng,
            )
        })
        .collect()
}

/// Bestes Individuum aus Referenzen auf EvalResults (für Gen 0).
fn best_from_refs<G: Genome>(pop: &[G], results: &[&EvalResult]) -> (G, f64) {
    let best = results
        .iter()
        .enumerate()
        .filter(|(_, r)| r.fitness.is_finite())
        .max_by(|a, b| a.1.fitness.partial_cmp(&b.1.fitness).unwrap());
    match best {
        Some((i, r)) => (pop[i].clone(), r.fitness),
        None => (pop[0].clone(), f64::NEG_INFINITY),
    }
}

/// Bestes Individuum aus owned EvalResults (für Gen 1+).
fn best_from_owned<G: Genome>(pop: &[G], results: &[EvalResult]) -> (G, f64) {
    let best = results
        .iter()
        .enumerate()
        .filter(|(_, r)| r.fitness.is_finite())
        .max_by(|a, b| a.1.fitness.partial_cmp(&b.1.fitness).unwrap());
    match best {
        Some((i, r)) => (pop[i].clone(), r.fitness),
        None => (pop[0].clone(), f64::NEG_INFINITY),
    }
}

fn print_header() {
    println!(
        "  {:>4} │  {:<9}  {:<4}  │  {:>9}  {:>9}  │  {}",
        "Gen", "Asset", "Int", "Score A", "Score B", "All-time Best"
    );
    println!("  ─────┼───────────────────┼────────────────────────┼──────────────────────");
}

fn print_gen(
    gen: u32,
    fa: f64,
    fb: f64,
    asset: &str,
    iv: &str,
    best: f64,
    best_gen: u32,
    is_new: bool,
) {
    let fa_str = if fa.is_finite() {
        format!("{fa:+9.4}")
    } else {
        "   -inf  ".into()
    };
    let fb_str = if fb.is_finite() {
        format!("{fb:+9.4}")
    } else {
        "  -inf   ".into()
    };
    let best_str = if best.is_finite() {
        format!("{best:+9.4}")
    } else {
        "   -inf  ".into()
    };
    let marker = if is_new { "  ◄" } else { "" };
    println!(
        "  {gen:>4} │  {asset:<9}  {iv:<4}  │  {fa_str}  {fb_str}  │  {best_str}  (Gen {best_gen:>4}){marker}"
    );
}

fn make_log(
    gen: u32,
    fa: f64,
    fb: f64,
    asset: String,
    interval: String,
    best: f64,
    best_gen: u32,
) -> GenerationLog {
    GenerationLog {
        generation: gen,
        fresh_a: fa,
        fresh_b: fb,
        asset,
        interval,
        all_time_best: best,
        best_gen,
    }
}
