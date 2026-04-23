use bench_bug::add;

#[test]
fn add_adds() {
    assert_eq!(add(2, 3), 5);
}
