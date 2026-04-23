use bench_feature::math::{is_even, triple};

#[test]
fn feature_math_works() {
    assert_eq!(triple(4), 12);
    assert!(is_even(6));
}
