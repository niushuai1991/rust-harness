import type {SessionEvent} from './types';

export type ShellTimelineNode={kind:'event';event:SessionEvent}|{kind:'shell';key:string;call?:SessionEvent;result?:SessionEvent};
export type ShellResult={kind:'output';stdout:string;stderr:string;exitCode:number}|{kind:'raw';text:string};

export function parseShellResult(content:unknown):ShellResult {
 const text=typeof content==='string'?content:content==null?'':JSON.stringify(content)??String(content);
 try {
  const value:unknown=JSON.parse(text);
  if(value&&typeof value==='object'){
   const data=value as Record<string,unknown>;
   if(typeof data.stdout==='string'&&typeof data.stderr==='string'&&typeof data.exit_code==='number'&&Number.isFinite(data.exit_code))return {kind:'output',stdout:data.stdout,stderr:data.stderr,exitCode:data.exit_code};
  }
 } catch { /* 旧记录、错误文本和截断预览保留原文 */ }
 return {kind:'raw',text};
}

export function projectShellEvents(events:SessionEvent[]):ShellTimelineNode[] {
 const calls=new Map<string,SessionEvent[]>(),results=new Map<string,SessionEvent[]>(),positions=new Map<SessionEvent,number>();
 const key=(event:SessionEvent,id:string)=>JSON.stringify([event.sessionId,id]);
 for(const [index,event] of events.entries()){
  positions.set(event,index);
  if(event.data.name!=='shell')continue;
  const id=event.kind==='tool.call'?event.data.id:event.kind==='tool.result'?event.data.callId:null;
  if(typeof id!=='string'||!id)continue;
  const target=event.kind==='tool.call'?calls:results,k=key(event,id);
  target.set(k,[...(target.get(k)??[]),event]);
 }
 const paired=new Map<SessionEvent,SessionEvent>(),used=new Set<SessionEvent>();
 for(const [id,entries] of calls){
  const found=results.get(id);
  if(entries.length===1&&found?.length===1&&positions.get(entries[0])!<positions.get(found[0])!){paired.set(entries[0],found[0]);used.add(found[0])}
 }
 const nodes:ShellTimelineNode[]=[];
 for(const event of events){
  if(used.has(event))continue;
  if(event.data.name==='shell'&&event.kind==='tool.call'){
   const result=paired.get(event);nodes.push(result?{kind:'shell',key:event.id,call:event,result}:{kind:'shell',key:event.id,call:event});
  } else if(event.data.name==='shell'&&event.kind==='tool.result')nodes.push({kind:'shell',key:event.id,result:event});
  else nodes.push({kind:'event',event});
 }
 return nodes;
}
