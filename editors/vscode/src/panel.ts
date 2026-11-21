import { randomBytes } from 'node:crypto';

/***
 * The panel's own page (docs/EDITOR.md §6, decision docair-p2c.15): a frame of the served documentation, forwarded out of a devcontainer, and a script that passes the page's messages on to the extension, only those from the frame's origin.
 *
 * @id      docair-vscode.panel
 * @kind    module
 * @part-of docair-vscode
 ***/

/** A nonce for the panel's one script. */
export function nonce(): string {
  return randomBytes(16).toString('base64');
}

const attr = (s: string) => s.replace(/&/g, '&amp;').replace(/"/g, '&quot;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

/***
 * The webview's HTML around the page at `frame`: only that frame and the one script with `nonce` are allowed; a message counts when it comes from the frame's origin and its type starts with `docair:`.
 *
 * @id      docair-vscode.panel.panel-html
 * @part-of docair-vscode.panel
 ***/
export function panelHtml(frame: string, nonce: string): string {
  const origin = new URL(frame).origin;
  return `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; frame-src ${attr(origin)}; style-src 'unsafe-inline'; script-src 'nonce-${attr(nonce)}';">
<style>html, body { margin: 0; padding: 0; height: 100%; overflow: hidden; } iframe { border: 0; width: 100%; height: 100%; display: block; }</style>
</head>
<body>
<iframe id="docair" title="docair" src="${attr(frame)}"></iframe>
<script nonce="${attr(nonce)}">
const vscode = acquireVsCodeApi();
const origin = ${JSON.stringify(origin).replace(/</g, '\\u003c')};
window.addEventListener('message', (e) => {
  if (e.origin !== origin || !e.data || typeof e.data.type !== 'string' || !e.data.type.startsWith('docair:')) return;
  vscode.postMessage(e.data);
});
</script>
</body>
</html>`;
}

/** The page's address in the panel: in the editor (`?editor=vscode`), and new each time so the frame loads it again. */
export function frameUrl(external: string, load: number): string {
  const u = new URL(external);
  u.searchParams.set('editor', 'vscode');
  u.searchParams.set('load', String(load));
  return u.toString();
}
