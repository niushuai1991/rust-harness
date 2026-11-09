import type {SessionEvent} from './types';

export interface TokenBudgetSnapshot {
 turnId:string;
 usedTokens:number;
 limitTokens:number;
 status:'active'|'exhausted'|'usageUnavailable';
}

/// 从运行时预算事件读取当前（最近一条用户消息）的累计用量快照。
export function tokenBudgetFromEvents(events:SessionEvent[]):TokenBudgetSnapshot|null{
 for(let index=events.length-1;index>=0;index-=1){
  const event=events[index];
  if(event.kind!=='token.budget')continue;
  const {turnId,usedTokens,limitTokens,status}=event.data;
  if(typeof turnId!=='string'||typeof usedTokens!=='number'||!Number.isSafeInteger(usedTokens)||usedTokens<0
    ||typeof limitTokens!=='number'||!Number.isSafeInteger(limitTokens)||limitTokens<=0
    ||(status!=='active'&&status!=='exhausted'&&status!=='usageUnavailable'))continue;
  return {turnId,usedTokens,limitTokens,status};
 }
 return null;
}
