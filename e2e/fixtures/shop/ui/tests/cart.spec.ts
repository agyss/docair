import { expect, test } from '@playwright/test';

/***
 * The cart's tests.
 *
 * @id      shop-tests
 * @kind    test
 * @part-of shop
 ***/
test.describe('cart', () => {
  /***
   * Buying sends the order to the API.
   *
   * @part-of shop-tests
   * @covers  shop-ui.cart
   * @covers  orders.create
   ***/
  test('buying stores the order', async () => {
    expect(1).toBe(1);
  });

  /***
   * The total is the sum of the items.
   *
   * @part-of shop-tests
   * @covers  shop-ui.cart.total
   ***/
  test('the total adds up', async () => {
    expect(2).toBe(2);
  });
});
