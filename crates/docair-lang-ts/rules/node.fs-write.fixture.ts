// Fixture for node.fs-write.yaml. Every `//>` line is a fact the rule must
// find, and it must find no others.

export function notes(shop: string) {
  fs.writeFileSync(path.join(shop, 'notes.md'), '# Notes\n');
}

export function out() {
  fs.mkdirSync('.out', { recursive: true });
}

//> writes file: from function notes
//> writes file:.out from function out
