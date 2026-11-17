import { HttpErrorResponse } from '@angular/common/http';
import { Injectable, computed, inject, signal } from '@angular/core';
import { Observable } from 'rxjs';
import { AskService, PatchAnswer, Reply, Turn } from './ask.service';

/** One entry of the conversation: the user's question, or the model's reply. */
export interface ChatEntry {
  id: number;
  role: 'user' | 'ai' | 'error';
  text: string;
  reply?: Reply;
  /** The sections this reply hid or showed, once it was applied: for Undo. */
  viewApplied?: { hid: string[]; showed: string[] };
  undone?: boolean;
  /** The edits of this reply as a patch, once reviewed or applied. */
  patch?: PatchAnswer;
  diffOpen?: boolean;
  applied?: boolean;
}

/***
 * The conversation with the model, kept across regenerated pages until a new upload.
 *
 * @id      docair-web.conversation
 * @kind    component
 * @part-of docair-web
 ***/
@Injectable({ providedIn: 'root' })
export class ConversationService {
  private readonly asker = inject(AskService);
  private next = 1;

  readonly entries = signal<ChatEntry[]>([]);
  readonly busy = signal(false);
  /** The earlier turns, as the model gets them. */
  readonly history = computed<Turn[]>(() => {
    const turns: Turn[] = [];
    let question = '';
    for (const e of this.entries()) {
      if (e.role === 'user') question = e.text;
      else if (e.role === 'ai' && question) {
        turns.push({ question, answer: e.text });
        question = '';
      }
    }
    return turns;
  });

  /***
   * Ask a question with the conversation so far; the reply is added when it arrives.
   *
   * @id      docair-web.conversation.ask
   * @part-of docair-web.conversation
   * @calls   docair-web.ask-service.ask (question, graph, history, context) -> Reply
   ***/
  ask(question: string, graph: object, context: string | undefined, done: (reply: Reply, entry: ChatEntry) => void): void {
    this.run(question, this.asker.ask(question, graph, this.history(), context), done);
  }

  /***
   * Ask for words on the lines that say nothing; the reply carries the words and the edits that put them into the code.
   *
   * @id      docair-web.conversation.word
   * @part-of docair-web.conversation
   * @calls   docair-web.ask-service.word (graph, scope) -> Reply
   ***/
  word(graph: object, scope: { id: string; name: string } | null, done: (reply: Reply, entry: ChatEntry) => void): void {
    const label = scope ? `Word the lines in ${scope.name}` : 'Word the lines that say nothing';
    this.run(label, this.asker.word(graph, scope?.id), done);
  }

  /***
   * Ask for glossary entries for the words the docs use without explaining them.
   *
   * @id      docair-web.conversation.glossary
   * @part-of docair-web.conversation
   * @calls   docair-web.ask-service.glossary (graph) -> Reply
   ***/
  glossary(graph: object, done: (reply: Reply, entry: ChatEntry) => void): void {
    this.run('Explain the terms of these docs', this.asker.glossary(graph), done);
  }

  private run(question: string, request: Observable<Reply>, done: (reply: Reply, entry: ChatEntry) => void): void {
    this.add({ role: 'user', text: question });
    this.busy.set(true);
    request.subscribe({
      next: (reply) => {
        const entry = this.add({ role: 'ai', text: reply.answer, reply });
        this.busy.set(false);
        done(reply, entry);
      },
      error: (err: HttpErrorResponse) => {
        this.add({ role: 'error', text: err.error?.error ?? err.message });
        this.busy.set(false);
      },
    });
  }

  update(id: number, patch: Partial<ChatEntry>): void {
    this.entries.update((list) => list.map((e) => (e.id === id ? { ...e, ...patch } : e)));
  }

  clear(): void {
    this.entries.set([]);
    this.busy.set(false);
  }

  private add(entry: Omit<ChatEntry, 'id'>): ChatEntry {
    const full = { ...entry, id: this.next++ };
    this.entries.update((list) => [...list, full]);
    return full;
  }
}
