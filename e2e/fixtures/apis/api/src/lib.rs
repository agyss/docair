/***
 * Store a new order and announce it.
 *
 * @id      orders.create
 * @part-of orders-api
 * @http    POST /api/orders
 * @emits   topic:order.created
 ***/
pub fn create_order() {}

/***
 * One order. The description names the parameter `{id}`.
 *
 * @id      orders.get
 * @part-of orders-api
 * @http    GET /api/orders/{orderId}
 ***/
pub fn get_order() {}

/***
 * Change an order. The route is misspelled on purpose: the description says `/api/orders/{id}`.
 *
 * @id      orders.update
 * @part-of orders-api
 * @http    PUT /api/order/{id}
 ***/
pub fn update_order() {}

/***
 * List the orders. The description leaves this route out, so docair warns (W115).
 *
 * @id      orders.list
 * @part-of orders-api
 * @http    GET /api/orders
 ***/
pub fn list_orders() {}

/***
 * Liveness probe. Left out of the description on purpose: `ignore` in docair.toml.
 *
 * @id      orders.health
 * @part-of orders-api
 * @http    GET /api/healthz
 ***/
pub fn health() {}
