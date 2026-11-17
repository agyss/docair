/***
 * Store a new order.
 *
 * @id      orders.create
 * @part-of orders-api
 * @http    POST /api/orders
 * @writes  orders-db (Order)
 * @flow    checkout 3 "the API stores the order"
 ***/
pub fn create_order() {}

/***
 * List the orders of a shopper.
 *
 * @id      orders.list
 * @part-of orders-api
 * @http    GET /api/orders
 ***/
pub fn list_orders() {}
