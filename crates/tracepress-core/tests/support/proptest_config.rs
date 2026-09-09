use proptest::test_runner::{Config, RngAlgorithm, RngSeed};

const PROPTEST_CASES: u32 = 64;

fn deterministic_proptest_config() -> Config {
    Config {
        cases: PROPTEST_CASES,
        rng_algorithm: RngAlgorithm::ChaCha,
        rng_seed: RngSeed::Fixed(0x5452_4143_4550_5245),
        ..Config::default()
    }
}
