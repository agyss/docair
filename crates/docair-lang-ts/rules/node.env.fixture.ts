// Fixture for node.env.yaml. Every `//>` line is a fact the rule must find,
// and it must find no others.

export function port() {
  return Number(process.env.PORT || 9555);
}

export function base() {
  return process.env['E2E_BASE_URL'];
}

// The whole environment, handed on: no one variable.
export function all() {
  return { ...process.env };
}

//> requests env:PORT by function port
//> requests env:E2E_BASE_URL by function base
