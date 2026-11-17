/***
 * The cart page: lists the items and places the order.
 *
 * @id      shop-ui.cart
 * @kind    component
 * @part-of shop-ui
 * @calls   http:POST /api/orders
 * @calls   http:POST /v1/charges "pays for the order"
 ***/
export class Cart {
  items: string[] = [];
}
