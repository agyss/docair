import { Component, DestroyRef, computed, inject, input, signal } from '@angular/core';

/**
 * What a generation waits for, in order: the dropped folder is read (`read`), the files go up
 * (`upload`), docair reads them on the server (`scan`), and the page comes back (`download`).
 * The page was laid out and drawn by docair already; it only shows.
 */
export type Phase = 'read' | 'upload' | 'scan' | 'download';

/** Where one generation is. */
export interface Progress {
  phase: Phase;
  /** Bytes (upload, download) or files (read) so far. */
  done?: number;
  /** Bytes in all, when known. */
  total?: number;
  /** Files in the upload. */
  files?: number;
  /** When this phase began (`Date.now()`). */
  since: number;
}

/** The three steps after Generate; reading a dropped folder comes before them. */
const STEPS: Phase[] = ['upload', 'scan', 'download'];

function size(bytes: number): string {
  if (bytes >= 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${bytes > 0 ? Math.max(1, Math.round(bytes / 1024)) : 0} kB`;
}

/***
 * Tells the user where a generation is: which of its three steps runs now, how far it got, and
 * for how long. A step that knows its size fills a bar; one that can only wait shows a moving
 * bar and counts the seconds.
 *
 * @id      docair-web.progress
 * @kind    component
 * @part-of docair-web
 ***/
@Component({
  selector: 'app-progress-line',
  templateUrl: './progress-line.html',
  styleUrl: './progress-line.css',
})
export class ProgressLine {
  readonly progress = input.required<Progress>();
  /** Small, for the slim bar above a page. */
  readonly compact = input(false);

  /** Ticks while the line is shown, so the seconds count up. */
  private readonly now = signal(Date.now());

  constructor() {
    const timer = setInterval(() => this.now.set(Date.now()), 500);
    inject(DestroyRef).onDestroy(() => clearInterval(timer));
  }

  /** 1–3, or 0 while the folder is read. */
  protected readonly step = computed(() => STEPS.indexOf(this.progress().phase) + 1);
  protected readonly seconds = computed(() => Math.max(0, Math.floor((this.now() - this.progress().since) / 1000)));
  /** 0–100 when the step knows how far it is; null while it can only wait. */
  protected readonly percent = computed(() => {
    const p = this.progress();
    if ((p.phase !== 'upload' && p.phase !== 'download') || !p.total) return null;
    return Math.min(100, Math.round((100 * (p.done ?? 0)) / p.total));
  });
  /** The step in words, without the ticking seconds. */
  protected readonly label = computed(() => {
    const p = this.progress();
    switch (p.phase) {
      case 'read':
        return `Reading the folder · ${p.done ?? 0} files so far`;
      case 'upload':
        return `Uploading ${p.files ?? 0} files · ${size(p.done ?? 0)} of ${size(p.total ?? 0)}`;
      case 'scan':
        return `docair reads the ${p.files ? `${p.files} ` : ''}files on the server`;
      case 'download':
        return `Receiving the page · ${size(p.done ?? 0)}${p.total ? ` of ${size(p.total)}` : ''}`;
    }
  });
  /** Seconds, for the steps that wait; a hint once a wait is long. */
  protected readonly time = computed(() => {
    const p = this.progress();
    const s = this.seconds();
    if (p.phase !== 'scan' || s < 1) return '';
    return ` · ${s} s${p.phase === 'scan' && s >= 10 ? ' · big projects take a while' : ''}`;
  });
}
