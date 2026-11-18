use crate::ledger::Entry;

/***
 * Compare the entries with the bank's payments; the ones that differ.
 *
 * @id      books.audit.compare
 * @part-of books.audit
 ***/
pub fn compare(entries: &[Entry], bank: &[i64]) -> Vec<usize> {
    entries
        .iter()
        .enumerate()
        .filter(|(i, e)| bank.get(*i) != Some(&e.cents))
        .map(|(i, _)| i)
        .collect()
}
