// Handwritten mirror of harness-contracts. Rust/Specta is the contract authority.
// 已知推理档位，语义强度从低到高（与 harness-contracts KNOWN_EFFORTS 对齐）。
export const KNOWN_EFFORTS=['disabled','minimal','low','medium','high','max'] as const;
export type Effort=(typeof KNOWN_EFFORTS)[number];
export type ExecutionMode='replica'|'worktree';
export interface SessionWorktree {path:string;branch:string;createdAt:string}
export interface WorkspaceInfo {id:string;sourcePath:string;replicaPath:string;createdAt:string;name:string;pinned:boolean;hidden:boolean;executionMode:ExecutionMode;worktrees:{[sessionId:string]:SessionWorktree}}
// 项目移除（隐藏）汇总（workspace_remove）与重开重新显示（workspace_reveal_hidden）。
export interface WorkspaceRemovalSummary {workspaceId:string;stoppedSessions:number}
export interface WorkspaceRevealSummary {workspaceId:string;revealed:number}
export interface SessionInfo {id:string;workspaceId:string;title:string;createdAt:string;updatedAt:string;parentId?:string|null;archivedAt?:string|null}
export interface SessionEvent {id:string;sessionId:string;seq:string;kind:string;data:Record<string,unknown>;createdAt:string}
export interface TimelinePage {sessionId:string;events:SessionEvent[];olderCursor:string|null;throughSeq:string;hasOlder:boolean}
export type LiveEvent = {type:'durable';event:SessionEvent}|{type:'delta';sessionId:string;requestId:string;index:number;text:string;reasoning:boolean}|{type:'status';sessionId:string;status:string};
export type ApiMode='chatCompletions'|'responses'|'anthropic';
export type ProviderCompatibility='standard'|'deepSeek'|'bigModel';
export type ProviderAuth='apiKey'|'chatGptOAuth';
export interface ProtocolEndpoint {apiMode:ApiMode;baseUrl:string}
export interface BuiltinModel {id:string;name:string;efforts:string[];contextWindow?:number|null}
export interface ProviderInfo {id:string;name:string;builtin:boolean;compatibility:ProviderCompatibility;auth:ProviderAuth;endpoints:ProtocolEndpoint[];defaultModel:string;credentialRef:string;proxy:string|null;enabled:boolean;builtinModels:BuiltinModel[];disabledModels:string[]}
export interface ModelConfig {id:string;name:string;baseUrl:string;model:string;credentialRef:string;proxy:string|null;providerId:string;compatibility:ProviderCompatibility;auth:ProviderAuth;apiMode:ApiMode;maxOutputTokens:number|null;contextWindow?:number|null;efforts:string[];effort:string|null;enabled:boolean;builtin:boolean}
export interface DefaultSelection {modelId:string;effort:string|null}
export type UsageCardKind='fiveHour'|'weekly'|'tools';
export interface ProviderUsageCard {kind:UsageCardKind;remainingPercent:number|null;resetAt:number|null}
export interface ProviderUsageSnapshot {planName:string|null;cards:ProviderUsageCard[]}
// ChatGPT 账号 OAuth（harness-contracts ChatGptLoginState/ChatGptAccountInfo）。
export type ChatGptLoginStatus='idle'|'pending'|'completed'|'failed'|'cancelled';
export interface ChatGptLoginState {status:ChatGptLoginStatus;email:string|null;planType:string|null;error:string|null}
export interface ChatGptAccountInfo {loggedIn:boolean;email:string|null;planType:string|null;expiresAt:number|null}
export interface ApprovalRequest {id:string;sessionId:string;tool:string;arguments:unknown;reason:string}
// ask_user 提问（harness-contracts AskOption/AskRequest/AskSource/AskAnswer）。
export interface AskOption {label:string;description?:string|null}
export interface AskRequest {id:string;sessionId:string;question:string;options:AskOption[]}
export type AskSource='selected'|'text'|'unattended'|'timeout';
export interface AskAnswer {text:string;source:AskSource}
export interface PluginInfo {id:string;name:string;version:string;kind:string;enabled:boolean;permissions:string[];status:string;description:string}
export interface FileChange {path:string;kind:string;beforeHash:string|null;afterHash:string|null;sourceHash:string|null;conflict:boolean;beforeText:string|null;afterText:string|null}
export interface WorkspaceFileDiff {path:string;unified:string|null}
export interface WorkspaceFilePreview {path:string;content:string;truncated:boolean;binary:boolean;sizeBytes:number}

export interface ArtifactMetadata {hash:string;name:string;mediaType:string;size:number}
export interface ArtifactChunk {encoding:'utf8'|'base64';content:string;nextOffset:number|null;totalSize:number}

// Orchestration payloads (serde JSON from harness-orchestration JobInfo/Schedule).
export interface JobInfo {id:string;command:string;status:string;cancelRequested:boolean;output:{stdout:string;stderr:string;exit_code:number}|null;error:unknown|null;createdAt:string}
export interface ScheduleInfo {id:string;sessionId:string;workspaceId:string;at:string;task:string;modelId:string;status:string}
export type AutomationKind='once'|'daily'|'weekly'|'monthly'|'yearly';
export interface AutomationRunInfo {id:string;scheduledAt:string|null;triggeredAt:string;trigger:'schedule'|'manual';status:'running'|'succeeded'|'failed';sessionId:string;error:string|null}
export interface AutomationInfo {id:string;title:string;prompt:string;sessionId:string|null;workspaceId:string;kind:AutomationKind;at:string|null;time:string|null;weekdays:number[]|null;monthDay:number|null;month:number|null;enabled:boolean;status:string;nextRunAt:string|null;lastRunAt:string|null;runCount:number;runs:AutomationRunInfo[];createdAt:string;updatedAt:string}

// Derived view for the plans/goals panel (harness-contracts PlanStateOverview); event log stays authoritative.
export interface PlanStateOverview {sessionId:string;sessionTitle:string;workspaceId:string;kind:string;data:Record<string,unknown>;updatedAt:string}

// 能力预设（harness-contracts PresetConfig/PresetInfo）：命名的会话能力配置包。
export const BUILTIN_PRESET_ID='builtin:default';
export const KNOWN_APPROVALS=['ask','edit','auto','never','guarded'] as const;
export type ApprovalPolicy=(typeof KNOWN_APPROVALS)[number];
export interface PresetConfig {id:string;name:string;description:string|null;modelId:string|null;effort:string|null;approval:string|null}
export type PresetInfo=PresetConfig&{broken:string|null};

// 技能（harness-contracts SkillInfo/SkillScope）：SKILL.md 目录行附启停标记。
export type SkillScope='project'|'user';
export interface SkillInfo {name:string;description:string|null;path:string;scope:SkillScope;enabled:boolean;brokenReason:string|null}
