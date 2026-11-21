import { ChildProcess, spawn } from 'node:child_process';

/***
 * The `docair serve` the extension starts for the workspace (docs/EDITOR.md §6, decision docair-p2c.14): on a free local port, its address read from its first line, its output passed on, stopped with the window.
 *
 * @id      docair-vscode.serve-process
 * @kind    module
 * @part-of docair-vscode
 ***/

/** How `docair serve` is started for a project folder: on 127.0.0.1, at a port the system picks. */
export function serveArgs(root: string): string[] {
  return ['serve', root, '--bind', '127.0.0.1:0'];
}

/***
 * Where `docair serve` listens, from what it printed: `docair serve: http://127.0.0.1:43123/ shows …` → `http://127.0.0.1:43123`.
 *
 * @id      docair-vscode.serve-process.address-of
 * @part-of docair-vscode.serve-process
 ***/
export function addressOf(said: string): string | undefined {
  return said.match(/docair serve: (http:\/\/[^\s/]+)\/ shows /)?.[1];
}

/***
 * One `docair serve` for one project folder.
 *
 * @id      docair-vscode.serve-process.serve-process
 * @part-of docair-vscode.serve-process
 ***/
export class ServeProcess {
  private proc?: ChildProcess;
  url?: string;

  constructor(
    private readonly binary: string,
    private readonly root: string,
    private readonly log: (text: string) => void,
  ) {}

  /** It runs. */
  get running(): boolean {
    return !!this.proc && this.proc.exitCode === null && this.proc.signalCode === null;
  }

  /***
   * Start `docair serve` and wait until it says where it listens; it first builds the documentation, which takes a while on a big project. Fails when the program is not found or stops before.
   *
   * @id      docair-vscode.serve-process.serve-process.start
   * @part-of docair-vscode.serve-process.serve-process
   * @calls   docair-cli.serve.cmd_serve "serves the workspace's project"
   * @calls   docair-vscode.serve-process.address-of "reads where it listens"
   ***/
  start(): Promise<string> {
    const proc = spawn(this.binary, serveArgs(this.root), { cwd: this.root, stdio: ['ignore', 'pipe', 'pipe'] });
    this.proc = proc;
    let said = '';
    return new Promise((resolve, reject) => {
      const look = (chunk: Buffer) => {
        const text = chunk.toString();
        this.log(text);
        if (this.url) return;
        said += text;
        const url = addressOf(said);
        if (url) {
          this.url = url;
          resolve(url);
        }
      };
      proc.stdout!.on('data', look);
      proc.stderr!.on('data', look);
      proc.on('error', (e: NodeJS.ErrnoException) =>
        reject(
          new Error(
            e.code === 'ENOENT'
              ? `${this.binary} is not found: install docair, or set docair.path to the program`
              : `${this.binary}: ${e.message}`,
          ),
        ),
      );
      proc.on('exit', (code, signal) => {
        if (!this.url) reject(new Error(`docair serve stopped (${signal ?? code}) before it served: ${said.trim().slice(-500)}`));
      });
    });
  }

  /** Stop it: an interrupt, as Ctrl-C would, and a kill when it has not stopped 3 s later. */
  stop(): void {
    const proc = this.proc;
    this.proc = undefined;
    this.url = undefined;
    if (!proc || proc.exitCode !== null) return;
    proc.kill('SIGINT');
    setTimeout(() => proc.exitCode === null && proc.kill('SIGKILL'), 3000).unref();
  }
}
