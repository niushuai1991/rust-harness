// Handwritten mirror of harness-contracts. Rust/Specta is the contract authority.
// 已知推理档位，语义强度从低到高（与 harness-contracts KNOWN_EFFORTS 对齐）。
export const KNOWN_EFFORTS=['disabled','minimal','low','medium','high','max'] as const;
export type Effort=(typeof KNOWN_EFFORTS)[number];
export interface WorkspaceInfo {id:string;sourcePath:string;replicaPath:string;createdAt:string}
export interface SessionInfo {id:string;workspaceId:string;title:string;createdAt:string;updatedAt:string;parentId?:string|null}
export interface SessionEvent {id:string;sessionId:string;seq:string;kind:string;data:Record<string,unknown>;createdAt:string}
export type LiveEvent = {type:'durable';event:SessionEvent}|{type:'delta';sessionId:string;requestId:string;index:number;text:string;reasoning:boolean}|{type:'status';sessionId:string;status:string};
export type ApiMode='chatCompletions'|'responses'|'anthropic';
export type ProviderCompatibility='standard'|'deepSeek'|'bigModel';
export interface ProtocolEndpoint {apiMode:ApiMode;baseUrl:string}
export interface BuiltinModel {id:string;name:string;efforts:string[]}
export interface ProviderInfo {id:string;name:string;builtin:boolean;compatibility:ProviderCompatibility;endpoints:ProtocolEndpoint[];defaultModel:string;credentialRef:string;proxy:string|null;enabled:boolean;builtinModels:BuiltinModel[];disabledModels:string[]}
export interface ModelConfig {id:string;name:string;baseUrl:string;model:string;credentialRef:string;proxy:string|null;providerId:string;compatibility:ProviderCompatibility;apiMode:ApiMode;maxOutputTokens:number|null;efforts:string[];effort:string|null;enabled:boolean;builtin:boolean}
export interface DefaultSelection {modelId:string;effort:string|null}
export type UsageCardKind='fiveHour'|'weekly'|'tools';
export interface ProviderUsageCard {kind:UsageCardKind;remainingPercent:number|null;resetAt:number|null}
export interface ProviderUsageSnapshot {planName:string|null;cards:ProviderUsageCard[]}
export interface ApprovalRequest {id:string;sessionId:string;tool:string;arguments:unknown;reason:string}
export interface PluginInfo {id:string;name:string;version:string;kind:string;enabled:boolean;permissions:string[];status:string;description:string}
export interface FileChange {path:string;kind:string;beforeHash:string|null;afterHash:string|null;sourceHash:string|null;conflict:boolean;beforeText:string|null;afterText:string|null}

export interface ArtifactMetadata {hash:string;name:string;mediaType:string;size:number}
export interface ArtifactChunk {encoding:'utf8'|'base64';content:string;nextOffset:number|null;totalSize:number}

// Orchestration payloads (serde JSON from harness-orchestration JobInfo/Schedule).
export interface JobInfo {id:string;command:string;status:string;cancelRequested:boolean;output:{stdout:string;stderr:string;exit_code:number}|null;error:unknown|null;createdAt:string}
export interface ScheduleInfo {id:string;sessionId:string;workspaceId:string;at:string;task:string;modelId:string;status:string}

// Derived view for the plans/goals panel (harness-contracts PlanStateOverview); event log stays authoritative.
export interface PlanStateOverview {sessionId:string;sessionTitle:string;workspaceId:string;kind:string;data:Record<string,unknown>;updatedAt:string}

// 能力预设（harness-contracts PresetConfig/PresetInfo）：命名的会话能力配置包。
export const BUILTIN_PRESET_ID='builtin:default';
export const KNOWN_APPROVALS=['ask','auto','never'] as const;
export type ApprovalPolicy=(typeof KNOWN_APPROVALS)[number];
export interface PresetConfig {id:string;name:string;description:string|null;modelId:string|null;effort:string|null;approval:string|null}
export type PresetInfo=PresetConfig&{broken:string|null};
