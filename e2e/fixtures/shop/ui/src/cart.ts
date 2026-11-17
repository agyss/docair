/***
 * The cart page: lists the items and sends the order.
 *
 * @id      shop-ui.cart
 * @kind    component
 * @part-of shop-ui
 * @calls   http:POST /api/orders (CreateOrder) -> Order
 * @creates data.order "the shopper clicks Buy"
 * @flow    checkout 2 "the cart posts the order"
 ***/
export class Cart {
  items: string[] = [];

  /***
   * Total of the cart, in cents.
   *
   * @id      shop-ui.cart.total
   * @part-of shop-ui.cart
   ***/
  total(): number {
    return this.items.length * 100;
  }
}
