// A fake OpenAI-compatible endpoint for the e2e tests: the same message always
// gets the same reply, so the tests are repeatable and cost nothing.
//
// POST /v1/chat/completions  answers by keywords of the last user message
// GET  /last                 the last request body, for assertions
// GET  /requests             the last 50 request bodies (tests run in parallel: find yours)
// GET  /health               ok
import http from 'node:http';

let last = null;
const requests = [];

function reply(messages) {
  // Words for lines (docair word, Word the lines): "checks <target>" for tests, "uses <target>" else.
  if ((messages[0]?.content || '').startsWith('You write the words on the lines')) {
    const lines = JSON.parse(messages[1].content.split('The lines:\n')[1]);
    return { lines: lines.map((l) => ({ n: l.n, text: `${l.kind === 'cover' ? 'checks' : 'uses'} ${l.to?.name ?? 'it'}` })) };
  }
  // Glossary entries (docair glossary, Explain the terms); the last one is not in the docs and is dropped.
  if ((messages[0]?.content || '').startsWith('You write the glossary')) {
    return {
      terms: [
        { term: 'orders table', aliases: [], definition: 'The table in Postgres that keeps every order.', see: ['orders-db'], link: '' },
        { term: 'cart page', aliases: [], definition: 'The page where the shopper sees the cart and buys.', see: ['shop-ui.cart'], link: 'https://example.com/cart' },
        { term: 'flux capacitor', aliases: [], definition: 'Not in these docs.', see: [], link: '' },
      ],
    };
  }
  const users = messages.filter((m) => m.role === 'user');
  const q = (users.at(-1)?.content || '').toLowerCase();
  // users[0] is the graph; the rest before the last are earlier turns.
  const earlier = Math.max(0, users.length - 2);
  if (q.includes("don't care") || q.includes('hide')) {
    return { answer: 'Hid the infrastructure.', view: { hide: ['infra'] } };
  }
  if (q.includes('missing') || q.includes('also reads')) {
    return {
      answer: 'Right: listing the orders reads the orders table, and the graph misses that.',
      changes: [{ node: 'orders.list', add: ['@reads orders-db -> Order'], why: 'Listing orders reads the orders table.' }],
    };
  }
  if (q.startsWith('and then')) {
    return { answer: `Turns so far: ${earlier}. Then the API stores the order in the orders table.` };
  }
  return {
    answer: 'Checkout goes from the cart to the API, which stores the order.',
    walkthrough: {
      title: 'Checkout',
      summary: 'The shopper buys; the cart posts the order; the API stores it.',
      steps: [
        { title: 'The shopper clicks Buy', text: 'The shopper presses Buy on the cart page.', nodes: ['shopper', 'shop-ui.cart'], edges: [{ from: 'shopper', to: 'shop-ui.cart' }] },
        { title: 'The cart posts the order', text: 'The cart sends `POST /api/orders` with the items.', nodes: ['shop-ui.cart', 'orders.create'], edges: [{ from: 'shop-ui.cart', to: 'orders.create' }] },
        { title: 'The API stores the order', text: 'create_order writes the order into the orders table.', nodes: ['orders.create', 'orders-db'], edges: [{ from: 'orders.create', to: 'orders-db' }] },
      ],
    },
  };
}

const server = http.createServer((req, res) => {
  if (req.method === 'GET' && req.url === '/health') return res.end('ok');
  if (req.method === 'GET' && req.url === '/last') {
    res.setHeader('content-type', 'application/json');
    return res.end(JSON.stringify(last));
  }
  if (req.method === 'GET' && req.url === '/requests') {
    res.setHeader('content-type', 'application/json');
    return res.end(JSON.stringify(requests));
  }
  if (req.method === 'POST' && req.url === '/v1/chat/completions') {
    let body = '';
    req.on('data', (c) => (body += c));
    req.on('end', () => {
      last = JSON.parse(body);
      requests.push(last);
      if (requests.length > 50) requests.shift();
      const content = JSON.stringify(reply(last.messages || []));
      res.setHeader('content-type', 'application/json');
      res.end(JSON.stringify({
        id: 'fake', object: 'chat.completion', model: last.model,
        choices: [{ index: 0, message: { role: 'assistant', content }, finish_reason: 'stop' }],
        usage: { prompt_tokens: Math.round(body.length / 4), completion_tokens: Math.round(content.length / 4) },
      }));
    });
    return;
  }
  res.statusCode = 404;
  res.end();
});
server.listen(Number(process.env.PORT || 9555), '127.0.0.1');
