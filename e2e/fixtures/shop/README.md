# shop

A shop: the shopper puts things in a cart, the API stores the order.

```docair
The shopper, in a browser.

@id      shopper
@kind    external
@part-of shop
@calls   shop-ui.cart (clicks Buy)
@flow    checkout 1 "the shopper clicks Buy"
```

```docair
The shopper's browser: the shop's web front end runs here.

@id      browser
@kind    external
@part-of shop
```

```docair
The order the shopper places: the items of the cart, as the cart sends them and the API stores them.

@id         data.order
@kind       data
@part-of    shop
@carried-as CreateOrder, Order
```

```docair
The orders table in Postgres.

@id      orders-db
@kind    store
@part-of cluster
@flow    keep 3 "the order is written"
```
