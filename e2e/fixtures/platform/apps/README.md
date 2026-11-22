# The shop's programs

The infrastructure has no doc blocks; these name what runs in it.

```docair
The shop's API: it answers the item requests.

@id      api
@kind    service
@part-of k8s.deploy.api
```

```docair
The shop's web front end.

@id      web
@kind    service
@part-of k8s.deploy.web
```

```docair
A shopper's browser: it asks the shop for its items.

@id      shopper
@kind    external
@part-of platform
@calls   k8s.ingress.shop (GET /api/items) -> items "asks for the items"
```
