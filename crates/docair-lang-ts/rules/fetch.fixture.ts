// Fixture for fetch.yaml. Every `//>` line is a fact the rule must find,
// and it must find no others.

export async function save(order: Order) {
  return fetch('/api/orders', { method: 'POST', body: JSON.stringify(order) });
}

export const remove = (id: string) => fetch(`/api/orders/${id}`, { method: "DELETE" });

// No method: fetch-get.yaml says these are GETs.
export async function load() {
  return fetch('/api/orders');
}

//> requests http POST /api/orders by function save
//> requests http DELETE /api/orders/${id} by const remove
