import { Component, DestroyRef, ElementRef, computed, effect, inject, input, output, signal, untracked, viewChild } from '@angular/core';
import { DomSanitizer } from '@angular/platform-browser';
import { ScanResult } from './scan.service';

/***
 * Shows the page docair wrote across the whole window, in a sandboxed frame, under a slim bar
 * with what docair found. The page is the documentation: laid out, drawn and linked by docair,
 * with every view baked in, and its AI asks the server, which keeps the upload as a project.
 * This app only shows it, by its address on the server, and waits for its `ready` message.
 *
 * @id      docair-web.result-view
 * @kind    component
 * @part-of docair-web
 * @calls   docair-viewer.doc_page.render_page (generated html) -> the documentation
 * @calls   docair-web.app.reset (new upload)
 * @flow    upload 15 "the generated page appears"
 ***/
@Component({
  selector: 'app-result-view',
  templateUrl: './result-view.html',
  styleUrl: './result-view.css',
})
export class ResultView {
  readonly result = input.required<ScanResult>();
  readonly closed = output<void>();

  private readonly sanitizer = inject(DomSanitizer);
  private readonly frame = viewChild<ElementRef<HTMLIFrameElement>>('frame');
  private url = '';

  /** The page runs in a sandboxed frame (scripts allowed, no access to this app): by its address when the server keeps the upload, so it reloads with edits applied. */
  protected readonly src = computed(() => {
    const at = this.result().page;
    return at ? this.sanitizer.bypassSecurityTrustResourceUrl(at) : null;
  });
  /** A server that keeps no upload: the page as it came. */
  protected readonly page = computed(() => this.sanitizer.bypassSecurityTrustHtml(this.result().html));
  /** The page says when its first picture shows. */
  protected readonly ready = signal(false);

  constructor() {
    effect(() => {
      this.result();
      untracked(() => this.ready.set(false));
    });
    const onMessage = (e: MessageEvent) => {
      if (!e.source || e.source !== this.frame()?.nativeElement.contentWindow) return;
      if (e.data?.type === 'docair:ready') this.ready.set(true);
    };
    window.addEventListener('message', onMessage);
    inject(DestroyRef).onDestroy(() => {
      window.removeEventListener('message', onMessage);
      if (this.url) URL.revokeObjectURL(this.url);
    });
  }

  /***
   * Open the generated page in a tab of its own.
   *
   * @id      docair-web.result-view.open
   * @part-of docair-web.result-view
   ***/
  protected open(): void {
    window.open(this.result().page ?? this.blobUrl(), '_blank', 'noopener');
  }

  /***
   * Save the generated page as one HTML file.
   *
   * @id      docair-web.result-view.download
   * @part-of docair-web.result-view
   ***/
  protected async download(): Promise<void> {
    // The page as it is now, with the edits applied since the upload.
    const at = this.result().page;
    const html = at ? await fetch(at).then((r) => (r.ok ? r.text() : this.result().html)).catch(() => this.result().html) : this.result().html;
    // Saved as a file, a kept upload's page still asks the server that keeps it (its AI, tidy).
    const served = at ? html.replace('<head>', `<head>\n<meta name="docair-serve" content="${new URL(at, location.href).href}">`) : html;
    const link = document.createElement('a');
    link.href = this.blobUrl(served);
    link.download = `${this.result().system}.docair.html`;
    link.click();
  }

  private blobUrl(html = this.result().html): string {
    if (this.url) URL.revokeObjectURL(this.url);
    this.url = URL.createObjectURL(new Blob([html], { type: 'text/html' }));
    return this.url;
  }
}
