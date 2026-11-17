import { HttpErrorResponse, HttpEventType } from '@angular/common/http';
import { Component, inject, signal } from '@angular/core';
import { ConversationService } from './conversation.service';
import { ResultView } from './result-view';
import { PickedFile, ScanResult, ScanService } from './scan.service';
import { UploadPanel } from './upload-panel';

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
  private readonly conversation = inject(ConversationService);

  protected readonly busy = signal(false);
  protected readonly progress = signal(0);
  protected readonly error = signal('');
  protected readonly result = signal<ScanResult | null>(null);
  /** The files of the current page, kept so edits can be applied to them. */
  protected readonly files = signal<PickedFile[]>([]);

  /***
   * Back to the upload page for the next project.
   *
   * @id      docair-web.app.reset
   * @part-of docair-web.app
   ***/
  protected reset(): void {
    this.result.set(null);
    this.files.set([]);
    this.error.set('');
    this.progress.set(0);
    this.conversation.clear();
  }

  /***
   * Send the picked files and show the page when the answer arrives.
   *
   * @id      docair-web.app.generate
   * @part-of docair-web.app
   * @calls   docair-web.scan-service.scan (picked files) -> upload progress, ScanResult
   * @calls   docair-web.result-view (ScanResult)
   ***/
  protected generate(files: PickedFile[]): void {
    this.files.set(files);
    this.busy.set(true);
    this.error.set('');
    this.progress.set(0);
    this.scanner.scan(files).subscribe({
      next: (event) => {
        if (event.type === HttpEventType.UploadProgress && event.total) {
          this.progress.set(Math.round((100 * event.loaded) / event.total));
        } else if (event.type === HttpEventType.Response) {
          this.result.set(event.body);
          this.busy.set(false);
        }
      },
      error: (err: HttpErrorResponse) => {
        this.error.set(err.error?.error ?? err.message);
        this.busy.set(false);
      },
    });
  }
}
