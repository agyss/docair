import { Component, ElementRef, effect, input, output, signal, viewChild } from '@angular/core';
import { ChatEntry } from './conversation.service';
import { Walkthrough } from './ask.service';

/** What is selected on the chart, sent along as context. */
export interface Selection {
  id: string;
  name: string;
  kind: string;
}

/***
 * The conversation panel at the right of the page: questions, replies, and what each reply can do:
 * play a walkthrough, hide or show sections (with Undo), propose edits (review, apply and regenerate, download).
 *
 * @id      docair-web.chat-panel
 * @kind    component
 * @part-of docair-web
 * @creates data.question "you type it"
 * @calls   docair-web.result-view.ask (question)
 ***/
@Component({
  selector: 'app-chat-panel',
  templateUrl: './chat-panel.html',
  styleUrl: './chat-panel.css',
})
export class ChatPanel {
  readonly entries = input.required<ChatEntry[]>();
  readonly busy = input(false);
  readonly model = input('');
  readonly context = input<Selection | null>(null);

  readonly asked = output<string>();
  readonly closed = output<void>();
  readonly play = output<Walkthrough>();
  readonly saveWalkthrough = output<Walkthrough>();
  readonly undo = output<ChatEntry>();
  readonly review = output<ChatEntry>();
  readonly apply = output<ChatEntry>();
  readonly downloadPatch = output<ChatEntry>();
  readonly clearContext = output<void>();
  /** Ask for words on the lines that say nothing (in the selected part, if one is selected). */
  readonly wordLines = output<void>();
  /** Ask for glossary entries for the words the docs do not explain yet. */
  readonly explainTerms = output<void>();

  protected readonly draft = signal('');
  private readonly log = viewChild<ElementRef<HTMLElement>>('log');

  protected readonly suggestions = [
    'Walk me through the main use case, from start to end.',
    'What is not documented yet? Document it.',
    "I don't care about the infrastructure.",
  ];

  constructor() {
    // Keep the newest message in view.
    effect(() => {
      this.entries();
      this.busy();
      const el = this.log()?.nativeElement;
      if (el) queueMicrotask(() => (el.scrollTop = el.scrollHeight));
    });
  }

  protected submit(event?: Event): void {
    event?.preventDefault();
    const q = this.draft().trim();
    if (!q || this.busy()) return;
    this.asked.emit(q);
    this.draft.set('');
  }

  protected keydown(event: KeyboardEvent): void {
    if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault();
      this.submit();
    }
  }

  protected send(text: string): void {
    this.draft.set(text);
    this.submit();
  }

  protected viewText(v: { hid: string[]; showed: string[] }): string {
    const parts = [];
    if (v.hid.length) parts.push(`Hid ${v.hid.join(', ')}`);
    if (v.showed.length) parts.push(`showed ${v.showed.join(', ')}`);
    return parts.join('; ') + '.';
  }

  protected diffLines(diff: string): { cls: string; text: string }[] {
    return diff.split('\n').map((text) => ({
      text,
      cls: text.startsWith('+++') || text.startsWith('---') || text.startsWith('diff ') ? 'file' : text.startsWith('+') ? 'add' : text.startsWith('-') ? 'rm' : text.startsWith('@@') ? 'hunk' : '',
    }));
  }
}
