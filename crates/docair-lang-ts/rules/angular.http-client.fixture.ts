// Fixture for angular.http-client.yaml. Every `//>` line is a fact the rule
// must find, and it must find no others.

import { HttpClient } from '@angular/common/http';
import { Injectable, inject } from '@angular/core';

@Injectable({ providedIn: 'root' })
export class ScanService {
  private readonly http = inject(HttpClient);

  scan(form: FormData) {
    return this.http.post<ScanResult>('/api/scan', form, { reportProgress: true });
  }

  status() {
    return this.http.get<AiStatus>('/api/ai');
  }

  /** Without a reply type, and with a template path. */
  order(id: string) {
    return this.http.delete(`/api/orders/${id}`);
  }
}

export class OldStyle {
  constructor(private client: HttpClient) {}

  load() {
    return this.client.get<Item[]>('/api/items');
  }
}

export class NotAClient {
  private readonly http = inject(Router);
  private readonly cache = new Map<string, string>();

  // `http` here is no HttpClient, and `cache` is a Map: no requests.
  go() {
    this.http.get('/not/a/request');
    this.cache.get('/not/a/request');
  }
}

//> requests http POST /api/scan -> ScanResult by scan
//> requests http GET /api/ai -> AiStatus by status
//> requests http DELETE /api/orders/${id} by order
//> requests http GET /api/items -> Item[] by load
