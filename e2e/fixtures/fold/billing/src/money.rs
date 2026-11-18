// No doc block here: docair finds these parts in the code only.

pub struct Amount(pub i64);

impl Amount {
    pub fn add(self, other: Amount) -> Amount {
        Amount(self.0 + other.0)
    }
}

pub fn round(cents: i64) -> i64 {
    cents
}
