// 会话上下文用量的纯计算（数据口径见 docs/specs/context-usage-indicator.md）。
// 「当前上下文」= 最近一次服务端报告的输入+输出 token 之和（与 codex last_token_usage 同口径）；
// compaction.summary 之前的历史已被清空，其 usage 全部作废。
// 缓存命中率 = Σ缓存读取 / Σ完整输入（对齐 zcode mainTurnCacheHitAggregate：分母用 total input，
// 不重复叠加缓存字段；累计范围与上下文占用一致，压缩点止步）。
import type {SessionEvent,UsageCardKind} from './types';

/// 额度卡片标签（与 ModelPanel 的额度卡共用）。
export const USAGE_CARD_LABELS:Record<UsageCardKind,string>={fiveHour:'5 小时剩余',weekly:'每周剩余',tools:'工具调用'};

/// 重置时间显示：5 小时卡当天只显示时刻，其余显示日期。
export function formatResetTime(resetAt:number|null,kind:UsageCardKind):string{
 if(resetAt==null)return'';
 const date=new Date(resetAt),suffix=' 重置';
 if(kind==='fiveHour'){const now=new Date();const sameDay=date.getFullYear()===now.getFullYear()&&date.getMonth()===now.getMonth()&&date.getDate()===now.getDate();
  if(sameDay)return`${String(date.getHours()).padStart(2,'0')}:${String(date.getMinutes()).padStart(2,'0')}${suffix}`;}
 return`${date.getMonth()+1}月${date.getDate()}日${suffix}`;
}

/// 从 assistant.message 事件的 data.usage 归一化上下文 token 数；无法识别时返回 null。
/// 字段探测代替协议传参：三种协议的 usage 原样落库，字段名互不相同。
export function usageTokensFromData(data:Record<string,unknown>|undefined):number|null{
 const usage=data?.usage;
 if(!usage||typeof usage!=='object')return null;
 const record=usage as Record<string,unknown>;
 const num=(key:string):number|null=>{
  const value=record[key];
  return typeof value==='number'&&Number.isFinite(value)?value:null;
 };
 // Anthropic 把输入拆为 input + 缓存读/写；Responses 与 ChatCompletions 的输入字段已含缓存。
 const cacheRead=num('cache_read_input_tokens'),cacheWrite=num('cache_creation_input_tokens');
 const input=num('input_tokens'),prompt=num('prompt_tokens');
 const output=num('output_tokens')??num('completion_tokens');
 if(input!=null)return input+(cacheRead??0)+(cacheWrite??0)+(output??0);
 if(prompt!=null)return prompt+(output??0);
 return null;
}

/// 从事件流（按 seq 升序）计算当前上下文占用：最后一条有效 usage 的 assistant.message；
/// 遇到 compaction.summary 即止——压缩后尚无新请求时返回 null（显示 0）。
export function contextTokensFromEvents(events:SessionEvent[]):number|null{
 for(let index=events.length-1;index>=0;index-=1){
  const event=events[index];
  if(event.kind==='compaction.summary')break;
  if(event.kind!=='assistant.message')continue;
  const tokens=usageTokensFromData(event.data);
  if(tokens!=null)return tokens;
 }
 return null;
}

/// 单条 usage 的缓存命中归一：返回 [缓存读取, 完整输入]，缺缓存字段或输入不可解析时 null。
/// 分母（完整输入）：Anthropic 的 input_tokens 不含缓存拆分，需补 cache_read/cache_creation；
/// Responses 与 ChatCompletions 的输入字段已含 cached，直接作分母。cache_creation 只进分母不进分子。
function cacheTokensFromUsage(usage:unknown):[number,number]|null{
 if(!usage||typeof usage!=='object')return null;
 const record=usage as Record<string,unknown>;
 const num=(container:Record<string,unknown>|null|undefined,key:string):number|null=>{
  if(!container)return null;
  const value=container[key];
  return typeof value==='number'&&Number.isFinite(value)?value:null;
 };
 // Anthropic：缓存读写在顶层；input_tokens 不含缓存，分母需补读+写。
 const anthropicRead=num(record,'cache_read_input_tokens');
 if(anthropicRead!=null){
  const input=num(record,'input_tokens');
  if(input==null)return null;
  const cacheWrite=num(record,'cache_creation_input_tokens')??0;
  return[anthropicRead,input+anthropicRead+cacheWrite];
 }
 // OpenAI 系：缓存命中嵌在 *_tokens_details.cached_tokens；输入字段已含 cached，直接作分母。
 const openaiRead=num(record.prompt_tokens_details as Record<string,unknown>|undefined,'cached_tokens')
  ??num(record.input_tokens_details as Record<string,unknown>|undefined,'cached_tokens');
 if(openaiRead!=null){
  const input=num(record,'input_tokens')??num(record,'prompt_tokens');
  if(input==null)return null;
  return[openaiRead,input];
 }
 return null;
}

/// 从事件流（按 seq 升序）累计缓存命中：压缩点之后每条带有效 usage 的 assistant.message
/// 逐条累加（同 zcode mainTurnCacheHitAggregate 的会话累计口径）；缺缓存字段的 usage 整条
/// 跳过（不进分子分母）。无有效数据或分母为 0 → null（前端隐藏该行）。
export function cacheHitFromEvents(events:SessionEvent[]):{cacheRead:number;totalInput:number}|null{
 let cacheRead=0,totalInput=0;
 for(let index=events.length-1;index>=0;index-=1){
  const event=events[index];
  if(event.kind==='compaction.summary')break;
  if(event.kind!=='assistant.message')continue;
  const usage=(event.data as Record<string,unknown>|undefined)?.usage;
  const pair=cacheTokensFromUsage(usage);
  if(pair==null)continue;
  cacheRead+=pair[0];
  totalInput+=pair[1];
 }
 return totalInput>0?{cacheRead,totalInput}:null;
}

/// 命中率显示：80 → "80%"，1/3 → "33.3%"（最多一位小数）。
export function formatHitRate(cacheRead:number,totalInput:number):string{
 const percent=Math.min(100,Math.max(0,cacheRead/totalInput*100));
 const rounded=Math.round(percent*10)/10;
 return `${Number.isInteger(rounded)?rounded:rounded.toFixed(1)}%`;
}

/// token 数缩写：950 → 950，62_100 → 62.1k，200_000 → 200k，1_250_000 → 1.3M。
export function formatTokens(value:number):string{
 if(value<1000)return String(Math.round(value));
 if(value<1_000_000){
  const thousands=value/1000;
  const text=thousands>=100?Math.round(thousands).toString():thousands.toFixed(1).replace(/\.0$/,'');
  return `${text}k`;
 }
 const millions=value/1_000_000;
 return `${millions.toFixed(1).replace(/\.0$/,'')}M`;
}
