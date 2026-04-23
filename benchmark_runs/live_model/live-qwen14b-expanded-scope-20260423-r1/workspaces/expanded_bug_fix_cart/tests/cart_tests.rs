use bench_cart::cart::{subtotal_cents, CartItem};

#[test]
fn subtotal_multiplies_price_by_quantity() {
    let items = vec![
        CartItem { sku: "a".to_string(), price_cents: 250, quantity: 2 },
        CartItem { sku: "b".to_string(), price_cents: 100, quantity: 3 },
    ];
    assert_eq!(subtotal_cents(&items), 800);
}
