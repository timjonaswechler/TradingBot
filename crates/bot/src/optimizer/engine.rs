use rand::SeedableRng;
use rand::Rng;
use rand::rngs::StdRng;
use rayon::prelude::*;

use crate::optimizer::evaluator::{evaluate, CandlePool, EvalConfig};
use crate::optimizer::genome::DualMacdGenome;

/// Configuration for the genetic optimizer.
pub struct OptimizerConfig {
    /// Number of individuals per group (default 25).
    pub population_size: usize,
    /// Maximum number of generations to run (default 100).
    pub max_generations: usize,
    /// Starting mutation magnitude ∈ [0.0, 1.0] (default 0.8).
    pub initial_mutation: f64,
    /// Per-generation multiplier applied to mutation magnitude (default 0.97).
    pub mutation_decay: f64,
    /// Evaluator configuration.
    pub eval: EvalConfig,
    /// Assets to train on; one is picked at random per generation.
    pub assets: Vec<String>,
    /// RNG seed for reproducibility.
    pub seed: u64,
}

impl Default for OptimizerConfig {
    fn default() -> Self {
        Self {
            population_size: 25,
            max_generations: 100,
            initial_mutation: 0.8,
            mutation_decay:   0.97,
            eval:             EvalConfig::default(),
            assets:           vec!["SPY".to_string()],
            seed:             0,
        }
    }
}

/// Per-generation log entry.
#[derive(Debug, Clone)]
pub struct GenerationLog {
    pub generation:       usize,
    pub best_fitness_a:   f64,
    pub best_fitness_b:   f64,
    pub all_time_best:    f64,
    pub asset:            String,
    pub mutation_magnitude: f64,
}

/// Final result returned after all generations have run.
#[derive(Debug, Clone)]
pub struct OptimizationResult {
    pub winner:       DualMacdGenome,
    pub best_fitness: f64,
    pub generations:  Vec<GenerationLog>,
}

/// Run the two-group competitive genetic optimizer.
///
/// Algorithm:
/// - Two independent populations (A and B) of `population_size` genomes each.
/// - Each generation: pick a random asset, evaluate both groups in parallel,
///   keep top half, mutate to fill bottom half.
/// - Mutation magnitude decays each generation by `mutation_decay`
///   but never falls below 0.05.
pub fn run(cfg: OptimizerConfig, pool: &CandlePool) -> OptimizationResult {
    let mut rng = StdRng::seed_from_u64(cfg.seed);

    // ── Initialise populations ────────────────────────────────────────────────
    let mut group_a: Vec<DualMacdGenome> = (0..cfg.population_size)
        .map(|_| DualMacdGenome::random(&mut rng))
        .collect();
    let mut group_b: Vec<DualMacdGenome> = (0..cfg.population_size)
        .map(|_| DualMacdGenome::random(&mut rng))
        .collect();

    let mut all_time_best_genome  = group_a[0].clone();
    let mut all_time_best_fitness = f64::NEG_INFINITY;
    let mut mutation_magnitude    = cfg.initial_mutation;
    let mut log: Vec<GenerationLog> = Vec::with_capacity(cfg.max_generations);

    for gen in 0..cfg.max_generations {
        // ── Pick random asset for this generation ─────────────────────────────
        let asset = cfg.assets[rng.gen_range(0..cfg.assets.len())].clone();

        // Pre-generate per-genome seeds from the main RNG so each parallel
        // worker gets a deterministic, non-overlapping stream.
        let seeds_a: Vec<u64> = (0..cfg.population_size).map(|_| rng.gen()).collect();
        let seeds_b: Vec<u64> = (0..cfg.population_size).map(|_| rng.gen()).collect();

        // ── Parallel evaluation ───────────────────────────────────────────────
        let results_a: Vec<f64> = group_a
            .par_iter()
            .zip(seeds_a.par_iter())
            .map(|(g, &seed)| {
                let mut local_rng = StdRng::seed_from_u64(seed);
                evaluate(g, pool, &asset, &cfg.eval, &mut local_rng).fitness
            })
            .collect();

        let results_b: Vec<f64> = group_b
            .par_iter()
            .zip(seeds_b.par_iter())
            .map(|(g, &seed)| {
                let mut local_rng = StdRng::seed_from_u64(seed);
                evaluate(g, pool, &asset, &cfg.eval, &mut local_rng).fitness
            })
            .collect();

        // ── Sort indices by fitness descending ────────────────────────────────
        let sorted_a = argsort_desc(&results_a);
        let sorted_b = argsort_desc(&results_b);

        let best_a = results_a[sorted_a[0]];
        let best_b = results_b[sorted_b[0]];

        // ── Track all-time best ───────────────────────────────────────────────
        if best_a > all_time_best_fitness {
            all_time_best_fitness = best_a;
            all_time_best_genome  = group_a[sorted_a[0]].clone();
        }
        if best_b > all_time_best_fitness {
            all_time_best_fitness = best_b;
            all_time_best_genome  = group_b[sorted_b[0]].clone();
        }

        // ── Evolve: keep top half, mutate bottom half from top half ───────────
        let half = cfg.population_size / 2;

        // Collect mutations before updating the groups (avoid borrow issues).
        let new_bottom_a: Vec<(usize, DualMacdGenome)> = (half..cfg.population_size)
            .map(|i| {
                let parent_idx = sorted_a[i % half];
                let child = group_a[parent_idx].mutate(mutation_magnitude, &mut rng);
                (sorted_a[i], child)
            })
            .collect();
        for (slot, child) in new_bottom_a {
            group_a[slot] = child;
        }

        let new_bottom_b: Vec<(usize, DualMacdGenome)> = (half..cfg.population_size)
            .map(|i| {
                let parent_idx = sorted_b[i % half];
                let child = group_b[parent_idx].mutate(mutation_magnitude, &mut rng);
                (sorted_b[i], child)
            })
            .collect();
        for (slot, child) in new_bottom_b {
            group_b[slot] = child;
        }

        // ── Decay mutation magnitude ──────────────────────────────────────────
        mutation_magnitude = (mutation_magnitude * cfg.mutation_decay).max(0.05);

        // ── Log & print ───────────────────────────────────────────────────────
        log.push(GenerationLog {
            generation:         gen,
            best_fitness_a:     best_a,
            best_fitness_b:     best_b,
            all_time_best:      all_time_best_fitness,
            asset:              asset.clone(),
            mutation_magnitude,
        });

        println!(
            "Gen {:3} | Mut {:.3} | Best A: {:7.2} | Best B: {:7.2} | All-time: {:7.2} | {}",
            gen, mutation_magnitude, best_a, best_b, all_time_best_fitness, asset,
        );
    }

    OptimizationResult {
        winner:       all_time_best_genome,
        best_fitness: all_time_best_fitness,
        generations:  log,
    }
}

// ─── Helper ───────────────────────────────────────────────────────────────────

/// Return indices that would sort `values` in descending order.
fn argsort_desc(values: &[f64]) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..values.len()).collect();
    indices.sort_by(|&a, &b| {
        values[b]
            .partial_cmp(&values[a])
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    indices
}
