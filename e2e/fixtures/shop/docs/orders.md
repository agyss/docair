# Orders

## How an order moves

The shopper clicks Buy. The cart posts `CreateOrder` to `POST /api/orders`, and
create_order writes the order into the orders table.
