import { HttpErrorResponse, HttpEventType } from '@angular/common/http';
import { Component, inject, signal } from '@angular/core';
import { Progress } from './progress-line';
import { ResultView } from './result-view';
import { PickedFile, ScanResult, ScanService } from './scan.service';
import { CHANGED, UploadPanel } from './upload-panel';

/***
 * The root component: the upload panel, then the result. Holds the state of
 * one generation: busy, progress, error, result.
 *
 * @id      docair-web.app
 * @kind    component
 * @part-of docair-web
 ***/
@Component({
  selector: 'app-root',
  imports: [UploadPanel, ResultView],
  templateUrl: './app.html',
  styleUrl: './app.css',
})
export class App {
  private readonly scanner = inject(ScanService);

  protected readonly busy = signal(false);
  /** Where the generation is: the files go up, docair reads them, the page comes back. */
  protected readonly progress = signal<Progress | null>(null);
  protected readonly error = signal('');
  protected readonly result = signal<ScanResult | null>(null);

  /***
   * Back to the upload page for the next project.
   *
   * @id      docair-web.app.reset
   * @part-of docair-web.app
   ***/
  protected reset(): void {
    this.result.set(null);
    this.error.set('');
    this.progress.set(null);
  }

  /***
   * Send the picked files and show the page when the answer arrives. Each step says how far it
   * is: the upload in bytes, docair's reading in seconds, the answer in bytes.
   *
   * @id      docair-web.app.generate
   * @part-of docair-web.app
   * @calls   docair-web.scan-service.scan (picked files) -> upload progress, ScanResult
   * @calls   docair-web.result-view (ScanResult)
   ***/
  protected generate(files: PickedFile[]): void {
    this.busy.set(true);
    this.error.set('');
    const count = files.length;
    const bytes = files.reduce((n, f) => n + f.file.size, 0);
    this.progress.set({ phase: 'upload', done: 0, total: bytes, files: count, since: Date.now() });
    this.scanner.scan(files).subscribe({
      next: (event) => {
        const now = Date.now();
        const p = this.progress();
        if (event.type === HttpEventType.UploadProgress) {
          // Everything sent: docair reads the files now.
          if (event.total && event.loaded < event.total) this.progress.set({ phase: 'upload', done: event.loaded, total: event.total, files: count, since: p?.since ?? now });
          else if (p?.phase === 'upload') this.progress.set({ phase: 'scan', files: count, since: now });
        } else if (event.type === HttpEventType.ResponseHeader) {
          this.progress.set({ phase: 'download', done: 0, total: Number(event.headers.get('content-length')) || undefined, since: now });
        } else if (event.type === HttpEventType.DownloadProgress) {
          this.progress.set({ phase: 'download', done: event.loaded, total: event.total ?? p?.total, since: p?.phase === 'download' ? p.since : now });
        } else if (event.type === HttpEventType.Response) {
          this.progress.set(null);
          this.result.set(event.body);
          this.busy.set(false);
        }
      },
      error: (err: HttpErrorResponse) => {
        const said: string = err.error?.error ?? err.message;
        const mb = `${(bytes / 1024 / 1024).toFixed(1)} MB`;
        // The multipart body broke: a file changed on disk while it was sent. Too big: the
        // server says its limit, the page what was picked.
        this.error.set(
          err.status === 413 ? `The upload (${count} files, ${mb}) is too big for this server: ${said}.` : /incomplete data/i.test(said) ? CHANGED : said,
        );
        this.busy.set(false);
        this.progress.set(null);
      },
    });
  }
}
