/// An entry is one payment in the ledger: who paid, how much and when.
pub struct Entry {
    pub payer: String,
    pub cents: i64,
}

pub struct Payment {
    pub payer: String,
    pub cents: i64,
}

/***
 * Record a payment: a new entry at the top of the ledger.
 *
 * @id      books.ledger.record
 * @part-of books.ledger
 ***/
pub fn record(p: Payment) -> Entry {
    Entry {
        payer: p.payer,
        cents: p.cents,
    }
}
