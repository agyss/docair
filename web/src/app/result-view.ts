import { Component, DestroyRef, ElementRef, computed, inject, input, output, signal, viewChild } from '@angular/core';
import { DomSanitizer } from '@angular/platform-browser';
import { firstValueFrom } from 'rxjs';
import { AiStatus, AskService, FileText, PatchAnswer, Reply, Walkthrough } from './ask.service';
import { ChatPanel, Selection } from './chat-panel';
import { ChatEntry, ConversationService } from './conversation.service';
import { PickedFile, ScanResult } from './scan.service';

/***
 * Shows the generated page across the whole window, in a sandboxed frame,
 * under a slim bar with what docair found; at the right, the conversation
 * with the model. Replies play in the page, hide sections in it, or edit the
 * picked files and generate the page again.
 *
 * @id      docair-web.result-view
 * @kind    component
 * @part-of docair-web
 * @calls   docair-viewer.page (generated html) -> interactive map
 * @calls   docair-web.app.reset (new upload)
 * @flow    upload 15 "the generated page appears"
 ***/
@Component({
  selector: 'app-result-view',
  imports: [ChatPanel],
  templateUrl: './result-view.html',
  styleUrl: './result-view.css',
})
export class ResultView {
  readonly result = input.required<ScanResult>();
  /** The files the page was generated from, as picked. */
  readonly files = input<PickedFile[]>([]);
  readonly busy = input(false);
  readonly closed = output<void>();
  /** Generate the page again from these files. */
  readonly regenerate = output<PickedFile[]>();

  private readonly sanitizer = inject(DomSanitizer);
  private readonly asker = inject(AskService);
  protected readonly chat = inject(ConversationService);
  private readonly frame = viewChild<ElementRef<HTMLIFrameElement>>('frame');
  private url = '';

  /** The page runs in a sandboxed frame (scripts allowed, no access to this app). */
  protected readonly page = computed(() => this.sanitizer.bypassSecurityTrustHtml(this.result().html));

  protected readonly ai = signal<AiStatus | null>(null);
  protected readonly chatOpen = signal(true);
  protected readonly selection = signal<Selection | null>(null);
  /** What the page hides and whether it shows tests, to keep across regenerated pages. */
  private view: { hidden: string[]; tests: boolean } | null = null;

  constructor() {
    this.asker.status().subscribe({
      next: (status) => this.ai.set(status),
      error: () => this.ai.set({ enabled: false }),
    });
    const onMessage = (e: MessageEvent) => {
      if (!e.source || e.source !== this.frame()?.nativeElement.contentWindow) return;
      const m = e.data;
      if (m?.type === 'docair:selected') {
        this.selection.set(m.id ? { id: m.id, name: m.name, kind: m.kind } : null);
      } else if (m?.type === 'docair:view-changed') {
        this.view = { hidden: m.hidden ?? [], tests: m.tests !== false };
      }
    };
    window.addEventListener('message', onMessage);
    inject(DestroyRef).onDestroy(() => window.removeEventListener('message', onMessage));
  }

  /** A regenerated page starts with the sections the user had hidden. */
  protected frameLoaded(): void {
    if (this.view) this.post({ type: 'docair:view', hide: this.view.hidden, tests: this.view.tests });
  }

  private post(message: object): void {
    this.frame()?.nativeElement.contentWindow?.postMessage(message, '*');
  }

  /***
   * Ask a question: the reply plays its walkthrough and applies its view change in the page at once.
   *
   * @id      docair-web.result-view.ask
   * @part-of docair-web.result-view
   * @calls   docair-web.conversation.ask (question, graph, context) -> Reply
   * @calls   docair-web.result-view.play "replays the reply's walkthrough"
   ***/
  protected ask(question: string): void {
    const sel = this.selection();
    const context = sel ? `${sel.id} (${sel.kind}, "${sel.name}")` : undefined;
    this.chat.ask(question, this.result().graph, context, (reply: Reply, entry: ChatEntry) => {
      if (reply.walkthrough) this.play(reply.walkthrough);
      if (reply.view) {
        this.post({ type: 'docair:view', hide: reply.view.hide, show: reply.view.show });
        this.chat.update(entry.id, { viewApplied: { hid: reply.view.hide, showed: reply.view.show } });
      }
    });
  }

  /***
   * Words for the lines: the model writes them, the page shows them at once (marked ✦), the edits put them into the code.
   *
   * @id      docair-web.result-view.word
   * @part-of docair-web.result-view
   * @calls   docair-web.conversation.word (graph, selected part) -> Reply
   * @calls   docair-viewer.wording (words) -> lines with words
   ***/
  protected wordLines(): void {
    const sel = this.selection();
    this.chat.word(this.result().graph, sel ? { id: sel.id, name: sel.name } : null, (reply: Reply) => {
      if (reply.wording?.length) this.post({ type: 'docair:wording', items: reply.wording });
    });
  }

  /***
   * Explain the terms: the model writes glossary entries, the page underlines the terms at once, the edit writes them into the glossary.
   *
   * @id      docair-web.result-view.glossary
   * @part-of docair-web.result-view
   * @calls   docair-web.conversation.glossary (graph) -> Reply
   * @calls   docair-viewer.glossary-preview (entries) -> terms on the page
   ***/
  protected explainTerms(): void {
    this.chat.glossary(this.result().graph, (reply: Reply) => {
      if (reply.glossary?.length) this.post({ type: 'docair:glossary', terms: reply.glossary });
    });
  }

  /***
   * Replay a walkthrough: hand it to the page in the frame, which plays it on the chart.
   *
   * @id      docair-web.result-view.play
   * @part-of docair-web.result-view
   * @calls   docair-viewer.receive (walkthrough) -> replay
   ***/
  protected play(w: Walkthrough): void {
    this.post({ type: 'docair:walkthrough', walkthrough: w, play: true });
  }

  protected undo(entry: ChatEntry): void {
    const v = entry.viewApplied;
    if (!v) return;
    this.post({ type: 'docair:view', hide: v.showed, show: v.hid });
    this.chat.update(entry.id, { undone: true });
  }

  /***
   * Save a walkthrough as `<id>.json`, the format of `docs/walkthroughs/`: refs only, no bound indices.
   *
   * @id      docair-web.result-view.save-walkthrough
   * @part-of docair-web.result-view
   ***/
  protected saveWalkthrough(w: Walkthrough): void {
    const stored = { ...w, steps: w.steps.map(({ path, ...step }) => step) };
    delete (stored as { file?: string }).file;
    this.save(`${w.id}.json`, JSON.stringify(stored, null, 2) + '\n', 'application/json');
  }

  /** The prefix of picked paths before the scanned root: `shop/` for a picked folder `shop`. */
  private prefix(): string {
    const paths = this.files().map((f) => f.path);
    const first = paths[0]?.split('/')[0];
    return first && paths.every((p) => p.startsWith(first + '/')) ? first + '/' : '';
  }

  /***
   * Turn a reply's edits into new file texts and a diff: the browser sends the files they touch, the server applies them.
   *
   * @id      docair-web.result-view.patch
   * @part-of docair-web.result-view
   * @calls   docair-web.ask-service.patch (changes, files) -> PatchAnswer
   ***/
  private async patchFor(entry: ChatEntry): Promise<PatchAnswer> {
    if (entry.patch) return entry.patch;
    const changes = entry.reply?.changes ?? [];
    const prefix = this.prefix();
    const wanted = new Set(changes.map((c) => c.file));
    const files: FileText[] = [];
    for (const f of this.files()) {
      const rel = f.path.slice(prefix.length);
      if (f.path.startsWith(prefix) && wanted.has(rel)) files.push({ path: rel, text: await f.file.text() });
    }
    const ids = ((this.result().graph as { nodes?: { id: string }[] }).nodes ?? []).map((n) => n.id);
    const patch = await firstValueFrom(this.asker.patch(changes, files, ids));
    this.chat.update(entry.id, { patch });
    return patch;
  }

  protected async review(entry: ChatEntry): Promise<void> {
    try {
      await this.patchFor(entry);
      this.chat.update(entry.id, { diffOpen: !entry.diffOpen });
    } catch (e) {
      this.chat.update(entry.id, { patch: { files: [], diff: '', applied: [], problems: [`review failed: ${(e as Error).message}`] } });
    }
  }

  /***
   * Apply the edits to the picked files and generate the page again, so the change shows at once.
   *
   * @id      docair-web.result-view.apply
   * @part-of docair-web.result-view
   * @calls   docair-web.result-view.patch "builds the new texts and diff"
   * @calls   docair-web.app.generate (edited files)
   ***/
  protected async apply(entry: ChatEntry): Promise<void> {
    const patch = await this.patchFor(entry);
    if (!patch.files.length) return;
    const prefix = this.prefix();
    const edited = new Map(patch.files.map((f) => [prefix + f.path, f.text]));
    const files: PickedFile[] = this.files().map((f) => {
      const text = edited.get(f.path);
      if (text === undefined) return f;
      edited.delete(f.path);
      return { path: f.path, file: new File([text], f.file.name, { type: f.file.type }) };
    });
    for (const [path, text] of edited) files.push({ path, file: new File([text], path.split('/').pop() || path) });
    this.chat.update(entry.id, { applied: true });
    this.regenerate.emit(files);
  }

  protected async downloadPatch(entry: ChatEntry): Promise<void> {
    const patch = await this.patchFor(entry);
    this.save('docair.patch', patch.diff, 'text/x-diff');
  }

  /***
   * Open the generated page in a tab of its own.
   *
   * @id      docair-web.result-view.open
   * @part-of docair-web.result-view
   ***/
  protected open(): void {
    window.open(this.blobUrl(), '_blank', 'noopener');
  }

  /***
   * Save the generated page as one HTML file.
   *
   * @id      docair-web.result-view.download
   * @part-of docair-web.result-view
   ***/
  protected download(): void {
    const link = document.createElement('a');
    link.href = this.blobUrl();
    link.download = `${this.result().system}.docair.html`;
    link.click();
  }

  private save(name: string, text: string, type: string): void {
    const link = document.createElement('a');
    link.href = URL.createObjectURL(new Blob([text], { type }));
    link.download = name;
    link.click();
    setTimeout(() => URL.revokeObjectURL(link.href), 1000);
  }

  private blobUrl(): string {
    if (this.url) URL.revokeObjectURL(this.url);
    this.url = URL.createObjectURL(new Blob([this.result().html], { type: 'text/html' }));
    return this.url;
  }
}
