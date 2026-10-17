use super::*;

lazy_static! {
    pub static ref TEST_LAYER: Layer = Layer {
        name: "TestLayer",
        hasher_seeds: (1, 2),
        traffic_allocations: HashMap::from([
            (TestExperiment::Control.get_group_id(), 10.0,),
            (TestExperiment::Experiment.get_group_id(), 10.0,),
            (FooExperiment::Control.get_group_id(), 5.0,),
            (FooExperiment::Experiment.get_group_id(), 5.0,),
        ]),
        bucket_ranges: vec![
            BucketRange::new(TestExperiment::Control, 0..100),
            BucketRange::new(TestExperiment::Experiment, 300..400),
            BucketRange::new(FooExperiment::Control, 500..550),
            BucketRange::new(FooExperiment::Experiment, 550..600),
        ],
    };
}

const TEST_EXPERIMENT_CONTROL: &str = "Control";
const TEST_EXPERIMENT_EXPERIMENT: &str = "Experiment";

#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq)]
pub enum TestExperiment {
    Control,
    Experiment,
}

impl Experiment<TestExperiment> for TestExperiment {
    fn name() -> &'static str {
        "TestExperiment"
    }

    fn variant(&self) -> &'static str {
        match self {
            Self::Control => TEST_EXPERIMENT_CONTROL,
            Self::Experiment => TEST_EXPERIMENT_EXPERIMENT,
        }
    }

    fn allow_user_overrides_in_stable() -> bool {
        false
    }
}

impl FromStr for TestExperiment {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            TEST_EXPERIMENT_CONTROL => Ok(Self::Control),
            TEST_EXPERIMENT_EXPERIMENT => Ok(Self::Experiment),
            _ => Err(anyhow::anyhow!(
                "Variant {} is not a valid group in TestExperiment",
                s
            )),
        }
    }
}

const FOO_EXPERIMENT_CONTROL: &str = "Control";
const FOO_EXPERIMENT_EXPERIMENT: &str = "Experiment";

#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq)]
pub enum FooExperiment {
    Control,
    Experiment,
}

impl Experiment<FooExperiment> for FooExperiment {
    fn name() -> &'static str {
        "FooExperiment"
    }

    fn layer() -> &'static Layer {
        &TEST_LAYER
    }

    fn variant(&self) -> &'static str {
        match self {
            Self::Control => FOO_EXPERIMENT_CONTROL,
            Self::Experiment => FOO_EXPERIMENT_EXPERIMENT,
        }
    }

    fn allow_user_overrides_in_stable() -> bool {
        true
    }
}

impl FromStr for FooExperiment {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            FOO_EXPERIMENT_CONTROL => Ok(Self::Control),
            FOO_EXPERIMENT_EXPERIMENT => Ok(Self::Experiment),
            _ => Err(anyhow::anyhow!(
                "Variant {} is not a valid group in FooExperiment",
                s
            )),
        }
    }
}

#[test]
fn test_create_experiment_layer_mappings() {
    let layers = vec![&*TEST_LAYER];
    create_experiment_layer_mappings(&layers);

    assert_eq!(EXPERIMENT_LAYER_MAPPINGS.len(), 2);
    assert_eq!(
        EXPERIMENT_LAYER_MAPPINGS
            .get(TestExperiment::name())
            .expect("Layer mapping should have been created for TestExperiment.")
            .name(),
        TEST_LAYER.name()
    );
    assert_eq!(
        EXPERIMENT_LAYER_MAPPINGS
            .get(FooExperiment::name())
            .expect("Layer mapping should have been created for FooExperiment.")
            .name(),
        TEST_LAYER.name()
    );
}
