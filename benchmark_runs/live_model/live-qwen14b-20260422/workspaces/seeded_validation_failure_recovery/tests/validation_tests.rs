use bench_validation::normalize;

#[test]
fn trims_and_lowercases() {
    assert_eq!(normalize("  HeLLo  "), "hello");
}
