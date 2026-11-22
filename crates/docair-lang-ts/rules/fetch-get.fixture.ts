// Fixture for fetch-get.yaml. Every `//>` line is a fact the rule must find,
// and it must find no others.

export async function load() {
  return fetch('/api/orders');
}

export async function loadOne(id: string) {
  return fetch(`/api/orders/${id}`, { headers: { accept: 'application/json' } });
}

// With a method: that's fetch.yaml's.
export async function save() {
  return fetch('/api/orders', { method: 'POST' });
}

//> requests http GET /api/orders by function load
//> requests http GET /api/orders/${id} by function loadOne
