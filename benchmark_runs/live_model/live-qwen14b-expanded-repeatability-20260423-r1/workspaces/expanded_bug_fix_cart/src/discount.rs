pub fn apply_discount_cents(subtotal: u32, discount: u32) -> u32 {
    subtotal.saturating_sub(discount)
}
