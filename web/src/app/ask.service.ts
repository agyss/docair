import { HttpClient } from '@angular/common/http';
import { Injectable, inject } from '@angular/core';
import { Observable } from 'rxjs';

/** Whether docair-server can ask a model (SPEC §10.5). */
export interface AiStatus {
  enabled: boolean;
  model?: string;
}

/** One step of a walkthrough (SPEC §10.3). */
export interface WalkStep {
  title: string;
  text: string;
  nodes: string[];
  edges: { from: string; to: string }[];
  path?: number[];
}

/** A question answered as steps through the graph. */
export interface Walkthrough {
  schema: string;
  id: string;
  question: string;
  title: string;
  summary: string;
  recorded?: { at: string; by: string; model?: string; endpoint?: string };
  steps: WalkStep[];
}

/** One proposed edit of a doc block, resolved to its file (SPEC §10.6). */
export interface Change {
  node: string;
  mode: 'edit' | 'insert' | 'append';
  file: string;
  line: number;
  add: string[];
  remove: string[];
  summary?: string;
  why: string;
}

/** Sections to hide and show (SPEC §11). */
export interface ViewChange {
  hide: string[];
  show: string[];
}

/** One earlier turn, sent along with a follow-up. */
export interface Turn {
  question: string;
  answer: string;
}

/** What docair-server answers for a question (SPEC §10.7). */
export interface Reply {
  answer: string;
  walkthrough?: Walkthrough;
  view?: ViewChange;
  changes: Change[];
  /** Words for lines, shown at once before they are applied (SPEC §10.8). */
  wording?: { from: string; to: string; text: string }[];
  /** New glossary entries, shown at once before they are applied (SPEC §13.4). */
  glossary?: { term: string; aliases: string[]; definition: string; see: string[]; link: string }[];
  problems: string[];
  seconds: number;
  usage: { input: number; output: number };
}

export interface FileText {
  path: string;
  text: string;
}

/** The edited files and the patch for the real repository. */
export interface PatchAnswer {
  files: FileText[];
  diff: string;
  applied: number[];
  problems: string[];
}

/***
 * Talks to docair-server about questions: whether asking is on, the question with the conversation so far, and edits.
 *
 * @id      docair-web.ask-service
 * @kind    component
 * @part-of docair-web
 ***/
@Injectable({ providedIn: 'root' })
export class AskService {
  private readonly http = inject(HttpClient);

  /***
   * Ask the server whether a model is configured.
   *
   * @id      docair-web.ask-service.status
   * @part-of docair-web.ask-service
   * @calls   host-port (GET /api/ai) -> AiStatus
   ***/
  status(): Observable<AiStatus> {
    return this.http.get<AiStatus>('/api/ai');
  }

  /***
   * Send a question with the graph of the page and the earlier turns; the server asks the model and checks the reply against the graph.
   *
   * @id      docair-web.ask-service.ask
   * @part-of docair-web.ask-service
   * @calls   host-port (POST /api/ask: question, graph, history, context) -> Reply
   ***/
  ask(question: string, graph: object, history: Turn[], context?: string): Observable<Reply> {
    return this.http.post<Reply>('/api/ask', { question, graph, history, context });
  }

  /***
   * Ask for words on the lines that say nothing, in the whole graph or one part of it.
   *
   * @id      docair-web.ask-service.word
   * @part-of docair-web.ask-service
   * @calls   host-port (POST /api/word: graph, scope) -> Reply
   ***/
  word(graph: object, scope?: string): Observable<Reply> {
    return this.http.post<Reply>('/api/word', { graph, scope });
  }

  /***
   * Ask the model to explain the words of the docs that the glossary does not explain yet.
   *
   * @id      docair-web.ask-service.glossary
   * @part-of docair-web.ask-service
   * @calls   host-port (POST /api/glossary: graph) -> Reply
   ***/
  glossary(graph: object): Observable<Reply> {
    return this.http.post<Reply>('/api/glossary', { graph });
  }

  /***
   * Send edits and the files they touch; the server answers with the new texts and a unified diff.
   *
   * @id      docair-web.ask-service.patch
   * @part-of docair-web.ask-service
   * @calls   host-port (POST /api/patch: changes, files) -> PatchAnswer
   ***/
  patch(changes: Change[], files: FileText[], ids: string[]): Observable<PatchAnswer> {
    return this.http.post<PatchAnswer>('/api/patch', { changes, files, ids });
  }
}
