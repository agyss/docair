/***
 * `docair serve`'s routes for the pair the page compares (docs/EDITOR.md §4): read it, set it and wait until it is compared, drop it.
 *
 * @id      docair-vscode.api
 * @kind    module
 * @part-of docair-vscode
 ***/

/** `GET /api/changes`: what the page compares; `old` null without a changes section, `new` null for the working tree. */
export interface ServedPair {
  old: string | null;
  new: string | null;
  base: string | null;
  comparing: boolean;
  error: string | null;
  version: number;
}

/** What the pair is called: `main → the working tree`; empty when the page compares nothing. */
export function pairLabel(p: { old: string | null; new: string | null }, names: Map<string, string> = new Map()): string {
  if (!p.old) return '';
  const name = (rev: string) => names.get(rev) ?? (/^[0-9a-f]{40}$/i.test(rev) ? rev.slice(0, 8) : rev);
  return `${name(p.old)} → ${p.new ? name(p.new) : 'the working tree'}`;
}

type FetchLike = (url: string, init?: { method?: string; headers?: Record<string, string>; body?: string }) => Promise<{
  ok: boolean;
  status: number;
  json(): Promise<unknown>;
}>;

/***
 * The routes of one `docair serve`.
 *
 * @id      docair-vscode.api.api
 * @part-of docair-vscode.api
 ***/
export class Api {
  constructor(
    readonly url: string,
    private readonly fetcher: FetchLike = fetch,
  ) {}

  private async call(method: string, body?: unknown): Promise<ServedPair> {
    const res = await this.fetcher(`${this.url}/api/changes`, {
      method,
      headers: body === undefined ? undefined : { 'content-type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const answer = (await res.json().catch(() => ({}))) as Partial<ServedPair> & { error?: string };
    if (!res.ok) throw new Error(answer.error || `docair serve answered ${res.status}`);
    return answer as ServedPair;
  }

  /***
   * What the page compares now.
   *
   * @id      docair-vscode.api.api.pair
   * @part-of docair-vscode.api.api
   * @calls   docair-server.project.changes (GET /api/changes) -> ServeChanges
   ***/
  pair(): Promise<ServedPair> {
    return this.call('GET');
  }

  /***
   * Compare `old` with `neu`, the working tree when null; serve checks both names at once and compares in the background.
   *
   * @id      docair-vscode.api.api.compare
   * @part-of docair-vscode.api.api
   * @calls   docair-server.project.compare (POST /api/changes: old, new) -> ServeChanges
   ***/
  compare(old: string, neu: string | null): Promise<ServedPair> {
    return this.call('POST', neu ? { old, new: neu } : { old });
  }

  /***
   * The page without its changes section.
   *
   * @id      docair-vscode.api.api.stop
   * @part-of docair-vscode.api.api
   * @calls   docair-server.project.stop_comparing (DELETE /api/changes) -> ServeChanges
   ***/
  stop(): Promise<ServedPair> {
    return this.call('DELETE');
  }

  /***
   * Wait until serve has compared the pair it was given: ask every `every` ms, at most `within` ms.
   *
   * @id      docair-vscode.api.api.compared
   * @part-of docair-vscode.api.api
   * @calls   docair-vscode.api.api.pair "asks whether it is done"
   ***/
  async compared(every = 500, within = 30 * 60 * 1000): Promise<ServedPair> {
    const until = Date.now() + within;
    for (;;) {
      const p = await this.pair();
      if (!p.comparing) return p;
      if (Date.now() > until) throw new Error('docair serve is still comparing; look at the docair output');
      await new Promise((r) => setTimeout(r, every));
    }
  }
}
