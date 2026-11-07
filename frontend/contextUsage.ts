// 会话上下文用量的纯计算（数据口径见 docs/specs/context-usage-indicator.md）。
// 「当前上下文」= 最近一次服务端报告的输入+输出 token 之和（与 codex last_token_usage 同口径）；
// compaction.summary 之前的历史已被清空，其 usage 全部作废。
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
