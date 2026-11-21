import assert from 'node:assert/strict';
import { test } from 'node:test';
import { frameUrl, panelHtml } from '../panel';

test("the panel allows only the page's frame and its own script", () => {
  const html = panelHtml('http://localhost:61234/?editor=vscode&load=1', 'n0nce');
  assert.match(html, /default-src 'none'; frame-src http:\/\/localhost:61234; style-src 'unsafe-inline'; script-src 'nonce-n0nce';/);
  assert.match(html, /<iframe id="docair" title="docair" src="http:\/\/localhost:61234\/\?editor=vscode&amp;load=1"><\/iframe>/);
  assert.match(html, /<script nonce="n0nce">/);
  assert.match(html, /const origin = "http:\/\/localhost:61234";/);
  assert.match(html, /e\.origin !== origin/, "a message from elsewhere is dropped");
  assert.match(html, /startsWith\('docair:'\)/);
});

test('the frame loads the page in the editor, anew each time', () => {
  assert.equal(frameUrl('http://localhost:61234/', 1), 'http://localhost:61234/?editor=vscode&load=1');
  assert.equal(frameUrl('http://localhost:61234/', 2), 'http://localhost:61234/?editor=vscode&load=2');
});
