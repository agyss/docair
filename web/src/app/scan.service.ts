import { HttpClient, HttpEvent } from '@angular/common/http';
import { Injectable, inject } from '@angular/core';
import { Observable } from 'rxjs';

/** A file the user picked, with its path inside the picked folder. */
export interface PickedFile {
  path: string;
  file: File;
}

/** One diagnostic, as docair-server reports it. */
export interface Diagnostic {
  code: string;
  severity: 'error' | 'warning';
  message: string;
  file?: string;
  line?: number;
}

/** What docair-server answers for one upload. */
export interface ScanResult {
  system: string;
  files: number;
  nodes: number;
  edges: number;
  flows: number;
  errors: number;
  warnings: number;
  diagnostics: Diagnostic[];
  html: string;
  /** The graph behind the page (docair.graph.json), sent back with a question. */
  graph: object;
}

/***
 * Talks to docair-server: sends the picked files and reports the upload's progress.
 *
 * @id      docair-web.scan-service
 * @kind    component
 * @part-of docair-web
 ***/
@Injectable({ providedIn: 'root' })
export class ScanService {
  private readonly http = inject(HttpClient);

  /***
   * Upload the files as one multipart request; each part is named after the
   * file's path in the picked folder, so the server can rebuild the folder.
   *
   * The request goes to the page's own origin. In the cluster that is the
   * published port of the kind node, and the Ingress routes `/api` to
   * docair-server.
   *
   * @id      docair-web.scan-service.scan
   * @part-of docair-web.scan-service
   * @calls   host-port (POST /api/scan: files) -> ScanResult
   * @flow    upload 3 "the browser posts the files to /api/scan"
   ***/
  scan(files: PickedFile[]): Observable<HttpEvent<ScanResult>> {
    const form = new FormData();
    for (const picked of files) {
      form.append('file', picked.file, picked.path);
    }
    return this.http.post<ScanResult>('/api/scan', form, { observe: 'events', reportProgress: true });
  }
}
