// Fixture for node.fs-read.yaml. Every `//>` line is a fact the rule must
// find, and it must find no others.

export function report(dir: string) {
  return JSON.parse(fs.readFileSync(path.join(dir, 'results.json'), 'utf8'));
}

export async function env() {
  return readFile('.env', 'utf8');
}

// Another object's readFile is not the file system's.
export function other(store: Store) {
  return store.readFile('x');
}

//> reads file: from function report
//> reads file:.env from function env
