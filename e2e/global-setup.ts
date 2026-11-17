import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import { docair, fixture, out } from './tests/helpers';

/** Generate the static pages the viewer tests open: the shop, and the shop with `[view] hide = ["infra"]`. */
export default function globalSetup(): void {
  if (!fs.existsSync(docair)) throw new Error(`${docair} is missing: run \`npm run build\` first`);
  fs.rmSync(out, { recursive: true, force: true });
  execFileSync(docair, ['scan', fixture('shop'), '--out', path.join(out, 'shop')]);
  execFileSync(docair, ['scan', fixture('platform'), '--out', path.join(out, 'platform')]);
  const hidden = path.join(out, 'shop-hidden-src');
  fs.cpSync(fixture('shop'), hidden, { recursive: true });
  fs.appendFileSync(path.join(hidden, 'docair.toml'), '\n[view]\nhide = ["infra"]\n');
  execFileSync(docair, ['scan', hidden, '--out', path.join(out, 'shop-hidden')]);
  const shown = path.join(out, 'shop-show-src');
  fs.cpSync(fixture('shop'), shown, { recursive: true });
  fs.appendFileSync(path.join(shown, 'docair.toml'), '\n[view]\nhide = ["infra"]\nshow = ["cluster"]\n');
  execFileSync(docair, ['scan', shown, '--out', path.join(out, 'shop-show')]);
}
