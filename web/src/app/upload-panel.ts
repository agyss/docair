import { Component, computed, input, output, signal } from '@angular/core';
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

/** All files under a dropped file or folder, with their paths. */
async function filesOf(entry: FileSystemEntry): Promise<PickedFile[]> {
  if (entry.isFile) {
    const file = await new Promise<File>((resolve, reject) => (entry as FileSystemFileEntry).file(resolve, reject));
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
  return (await Promise.all(entries.map(filesOf))).flat();
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
  templateUrl: './upload-panel.html',
  styleUrl: './upload-panel.css',
})
export class UploadPanel {
  readonly busy = input(false);
  readonly progress = input(0);
  readonly generate = output<PickedFile[]>();

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
  protected pick(event: Event): void {
    const input = event.target as HTMLInputElement;
    this.take(Array.from(input.files ?? []).map((file) => ({ path: file.webkitRelativePath || file.name, file })));
    input.value = '';
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
    this.take((await Promise.all(entries.map(filesOf))).flat());
  }

  protected over(event: DragEvent): void {
    event.preventDefault();
    this.dragging.set(true);
  }

  private take(picked: PickedFile[]): void {
    const kept = picked.filter((p) => useful(p.path) && p.file.size <= MAX_FILE_BYTES);
    this.skipped.set(picked.length - kept.length);
    this.files.set(kept);
  }
}
