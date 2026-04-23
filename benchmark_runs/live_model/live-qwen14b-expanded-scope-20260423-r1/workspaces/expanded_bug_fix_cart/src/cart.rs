#[derive(Debug, Clone)]
pub struct CartItem {
    pub sku: String,
    pub price_cents: u32,
    pub quantity: u32,
}

pub fn subtotal_cents(items: &[CartItem]) -> u32 {
    items.iter().map(|item| item.quantity).sum()
}
