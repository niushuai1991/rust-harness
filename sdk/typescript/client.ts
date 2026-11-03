export type ProviderCompatibility = 'standard' | 'deepSeek' | 'bigModel';
export type ApiMode = 'chatCompletions' | 'responses' | 'anthropic';
export interface ProtocolEndpoint {apiMode: ApiMode; baseUrl: string}
export interface ProviderInfo {compatibility: ProviderCompatibility; id: string; name: string; builtin: boolean; endpoints: ProtocolEndpoint[]; defaultModel: string; credentialRef: string; proxy: string | null}
export interface ModelConfig {compatibility: ProviderCompatibility; id: string; name: string; providerId: string; apiMode: ApiMode; model: string; maxOutputTokens: number | null; baseUrl: string; credentialRef: string; proxy: string | null}

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
  listProviders() {return this.call<ProviderInfo[]>('provider.list');}
  saveProvider(config: ProviderInfo, secret?: string) {return this.call<ProviderInfo>('provider.save', {config, secret});}
  copyProvider(id: string) {return this.call<ProviderInfo>('provider.copy', {id});}
  removeProvider(id: string) {return this.call<null>('provider.remove', {id});}
  listModels() {return this.call<ModelConfig[]>('model.list');}
  saveModel(config: ModelConfig, secret?: string) {return this.call<ModelConfig>('model.save', {config, secret});}
  removeModel(modelId: string) {return this.call<null>('model.remove', {modelId});}
  testModel(modelId: string) {return this.call<string>('model.test', {modelId});}
  close() {this.process.stdin.end();}
}
