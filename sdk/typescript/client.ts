import {spawn, type ChildProcessWithoutNullStreams} from 'node:child_process';
import {createInterface} from 'node:readline';

/** Local process SDK. Approval decisions remain explicit caller actions. */
export class Harness {
  readonly process: ChildProcessWithoutNullStreams;
  private sequence = 0;
  private pending = new Map<number, {resolve(value: unknown): void; reject(error: Error): void}>();
  onEvent?: (event: unknown) => void;
  constructor(executable = 'harness', dataDir?: string) {
    this.process = spawn(executable, [...(dataDir ? ['--data-dir', dataDir] : []), 'rpc']);
    this.process.stderr.pipe(process.stderr);
    createInterface({input: this.process.stdout}).on('line', line => {
      const message = JSON.parse(line);
      if (message.id !== undefined) {
        const task = this.pending.get(message.id); this.pending.delete(message.id);
        if (message.error) task?.reject(new Error(JSON.stringify(message.error)));
        else task?.resolve(message.result);
      } else this.onEvent?.(message);
    });
    const fail = (error: Error) => {for (const task of this.pending.values()) task.reject(error); this.pending.clear();};
    this.process.on('error', fail).on('exit', () => fail(new Error('Rust Harness disconnected')));
  }
  call<T = unknown>(method: string, params: unknown = {}): Promise<T> {
    const id = ++this.sequence;
    return new Promise<T>((resolve, reject) => {
      this.pending.set(id, {resolve: value => resolve(value as T), reject});
      this.process.stdin.write(JSON.stringify({jsonrpc:'2.0', id, method, params})+'\n', error => {
        if (error) { this.pending.delete(id); reject(error); }
      });
    });
  }
  close() {this.process.stdin.end();}
}
