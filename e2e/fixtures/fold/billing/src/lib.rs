pub mod money;

/***
 * Bill an order: what it costs, in cents, with the tax.
 *
 * @id      billing.bill
 * @part-of billing
 * @calls   billing.tax "adds the tax"
 ***/
pub fn bill(items: &[i64]) -> i64 {
    tax(items.iter().sum())
}

pub fn tax(cents: i64) -> i64 {
    cents + cents / 5
}

pub fn fee() -> i64 {
    30
}
