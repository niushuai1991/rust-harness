// Handwritten mirror of harness-contracts. Rust/Specta is the contract authority.
export interface WorkspaceInfo {id:string;sourcePath:string;replicaPath:string;createdAt:string}
export interface SessionInfo {id:string;workspaceId:string;title:string;createdAt:string;updatedAt:string;parentId?:string|null}
export interface SessionEvent {id:string;sessionId:string;seq:string;kind:string;data:Record<string,unknown>;createdAt:string}
export type LiveEvent = {type:'durable';event:SessionEvent}|{type:'delta';sessionId:string;requestId:string;index:number;text:string;reasoning:boolean}|{type:'status';sessionId:string;status:string};
export type ApiMode='chatCompletions'|'responses'|'anthropic';
export type ProviderCompatibility='standard'|'deepSeek'|'bigModel';
export interface ProtocolEndpoint {apiMode:ApiMode;baseUrl:string}
export interface ProviderInfo {id:string;name:string;builtin:boolean;compatibility:ProviderCompatibility;endpoints:ProtocolEndpoint[];defaultModel:string;credentialRef:string;proxy:string|null}
export interface ModelConfig {id:string;name:string;baseUrl:string;model:string;credentialRef:string;proxy:string|null;providerId:string;compatibility:ProviderCompatibility;apiMode:ApiMode;maxOutputTokens:number|null}
export interface ApprovalRequest {id:string;sessionId:string;tool:string;arguments:unknown;reason:string}
export interface PluginInfo {id:string;name:string;version:string;kind:string;enabled:boolean;permissions:string[];status:string;description:string}
export interface FileChange {path:string;kind:string;beforeHash:string|null;afterHash:string|null;sourceHash:string|null;conflict:boolean;beforeText:string|null;afterText:string|null}

export interface ArtifactMetadata {hash:string;name:string;mediaType:string;size:number}
export interface ArtifactChunk {encoding:'utf8'|'base64';content:string;nextOffset:number|null;totalSize:number}
