/***
 * Ledger: the record of all payments, newest first.
 *
 * @id      books.ledger
 * @part-of books
 ***/
pub mod ledger;

/***
 * Checks the ledger every night: each entry against the bank's list of payments.
 *
 * @id      books.audit
 * @part-of books
 ***/
pub mod audit;
