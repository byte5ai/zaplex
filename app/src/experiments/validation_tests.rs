use super::*;
use std::collections::HashSet;

#[test]
fn test_no_experiments_in_multiple_layers() {
    let mut prev_seen_experiments: HashSet<String> = HashSet::new();
    for layer in LAYERS.iter() {
        // Set of experiments in the current layer
        let mut cur_experiments: HashSet<String> = HashSet::new();
        for group_id in layer.traffic_allocations.keys() {
            // group_id will follow format "{experiment_name}::{variant_name}"
            let experiment_name = group_id.to_string().split_once(':').unwrap().0.to_owned();
            assert!(
                !prev_seen_experiments.contains(&experiment_name),
                "Experiment {experiment_name} is used in multiple layers"
            );
            cur_experiments.insert(experiment_name);
        }
        prev_seen_experiments.extend(cur_experiments);
    }
}
