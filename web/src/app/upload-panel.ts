import { Component, computed, input, output, signal } from '@angular/core';
import { Progress, ProgressLine } from './progress-line';
import { PickedFile } from './scan.service';

/** Folders that only make an upload big: dependencies, build output, VCS data, provider caches. */
const SKIPPED = ['node_modules', '.git', 'target', 'dist', '.angular', '.docair', '.terraform'];

/** docair reads source text; bigger files are binaries or generated data. */
const MAX_FILE_BYTES = 2 * 1024 * 1024;

/** Files that hold secrets: never uploaded. `.env.example` is fine. */
function secret(name: string): boolean {
  return (
    (/^\.env(\..+)?$/.test(name) && name !== '.env.example') ||
    /\.tfstate(\..+)?$/.test(name) ||
    /\.tfvars(\.json)?$/.test(name)
  );
}

function useful(path: string): boolean {
  const parts = path.split('/');
  return !parts.some((part) => SKIPPED.includes(part)) && !secret(parts[parts.length - 1]);
}

/** What to say when a picked file cannot be read: it changed or went away after it was picked. */
export const CHANGED = 'A file changed while it was read; pick the folder again.';

/**
 * The file's bytes as they are now, in memory: a file that changes on disk after this cannot break
 * the upload (the browser would send it as it is at send time, and the sizes would no longer agree).
 */
async function kept(f: PickedFile): Promise<PickedFile> {
  const bytes = await f.file.arrayBuffer();
  return { path: f.path, file: new File([bytes], f.file.name, { type: f.file.type, lastModified: f.file.lastModified }) };
}

/** All files under a dropped file or folder, with their paths; `found` hears of each one. */
async function filesOf(entry: FileSystemEntry, found: () => void = () => {}): Promise<PickedFile[]> {
  if (entry.isFile) {
    const file = await new Promise<File>((resolve, reject) => (entry as FileSystemFileEntry).file(resolve, reject));
    found();
    return [{ path: entry.fullPath.replace(/^\//, ''), file }];
  }
  if (!entry.isDirectory || !useful(entry.fullPath)) {
    return [];
  }
  const reader = (entry as FileSystemDirectoryEntry).createReader();
  const entries: FileSystemEntry[] = [];
  for (;;) {
    const batch = await new Promise<FileSystemEntry[]>((resolve, reject) => reader.readEntries(resolve, reject));
    if (!batch.length) break;
    entries.push(...batch);
  }
  return (await Promise.all(entries.map((e) => filesOf(e, found)))).flat();
}

/***
 * Lets the user pick a folder or single files, by dialog or by drag and drop,
 * and hands them to the app. Dependency and build folders are left out, and so
 * are files with secrets: `.env`, Terraform state and `.tfvars`.
 *
 * @id      docair-web.upload-panel
 * @kind    component
 * @part-of docair-web
 * @creates data.files "you pick a folder"
 * @calls   docair-web.app.generate (picked files)
 * @flow    upload 2 "the user picks a folder and clicks Generate"
 ***/
@Component({
  selector: 'app-upload-panel',
  imports: [ProgressLine],
  templateUrl: './upload-panel.html',
  styleUrl: './upload-panel.css',
})
export class UploadPanel {
  readonly busy = input(false);
  /** Where the generation is, while it runs. */
  readonly progress = input<Progress | null>(null);
  readonly generate = output<PickedFile[]>();

  /** A dropped folder being read: how many files so far. */
  protected readonly reading = signal<Progress | null>(null);
  /** Why the picked files could not be taken. */
  protected readonly problem = signal('');

  protected readonly files = signal<PickedFile[]>([]);
  protected readonly skipped = signal(0);
  protected readonly dragging = signal(false);
  protected readonly folder = computed(() => this.files()[0]?.path.split('/')[0] ?? '');
  protected readonly size = computed(() => {
    const bytes = this.files().reduce((n, f) => n + f.file.size, 0);
    return bytes > 1024 * 1024 ? `${(bytes / 1024 / 1024).toFixed(1)} MB` : `${Math.ceil(bytes / 1024)} kB`;
  });

  /***
   * Take the files from the folder or file dialog.
   *
   * @id      docair-web.upload-panel.pick
   * @part-of docair-web.upload-panel
   ***/
  protected async pick(event: Event): Promise<void> {
    const input = event.target as HTMLInputElement;
    const picked = Array.from(input.files ?? []).map((file) => ({ path: file.webkitRelativePath || file.name, file }));
    input.value = '';
    await this.take(picked);
  }

  /***
   * Take what was dropped on the drop zone, walking into dropped folders.
   *
   * @id      docair-web.upload-panel.drop
   * @part-of docair-web.upload-panel
   ***/
  protected async drop(event: DragEvent): Promise<void> {
    event.preventDefault();
    this.dragging.set(false);
    const entries = Array.from(event.dataTransfer?.items ?? [])
      .map((item) => item.webkitGetAsEntry())
      .filter((entry): entry is FileSystemEntry => entry !== null);
    const since = Date.now();
    let found = 0;
    this.reading.set({ phase: 'read', done: 0, since });
    try {
      await this.take((await Promise.all(entries.map((e) => filesOf(e, () => this.reading.set({ phase: 'read', done: ++found, since }))))).flat());
    } catch {
      this.files.set([]);
      this.problem.set(CHANGED);
    } finally {
      this.reading.set(null);
    }
  }

  protected over(event: DragEvent): void {
    event.preventDefault();
    this.dragging.set(true);
  }

  /***
   * Keep the useful files, with their bytes as they are now, so the upload sends what was picked.
   *
   * @id      docair-web.upload-panel.take
   * @part-of docair-web.upload-panel
   ***/
  private async take(picked: PickedFile[]): Promise<void> {
    const wanted = picked.filter((p) => useful(p.path) && p.file.size <= MAX_FILE_BYTES);
    this.problem.set('');
    this.files.set([]);
    const since = Date.now();
    let read = 0;
    this.reading.set({ phase: 'read', done: 0, since });
    try {
      const files = await Promise.all(
        wanted.map(async (f) => {
          const k = await kept(f);
          this.reading.set({ phase: 'read', done: ++read, since });
          return k;
        }),
      );
      this.skipped.set(picked.length - files.length);
      this.files.set(files);
    } catch {
      this.problem.set(CHANGED);
    } finally {
      this.reading.set(null);
    }
  }
}
