import {diffStats,parseUnifiedDiff} from './DiffView';
import type {SessionEvent} from './types';

export type FileEditLabel='写入'|'编辑'|'修改';
export interface FileEditGroup{path:string;events:SessionEvent[]}
export type FileEditNode={kind:'event';event:SessionEvent}|{kind:'group';key:string;group:FileEditGroup};
export interface FileEditSummary{path:string;label:FileEditLabel;operations:number;pending:number;failed:number;add:number;del:number;hasStats:boolean;results:SessionEvent[]}

function editName(event:SessionEvent,callNames:Map<string,string>):'write'|'edit'|null {
 const name=event.data.name;
 if(typeof name==='string'&&name)return name==='write'||name==='edit'?name:null;
 if(event.kind!=='tool.result')return null;
 const callId=typeof event.data.callId==='string'?event.data.callId:'';
 const inherited=callNames.get(callId);
 return inherited==='write'||inherited==='edit'?inherited:null;
}

function editPath(event:SessionEvent,callPaths:Map<string,string>):string|null {
 const args=typeof event.data.arguments==='object'&&event.data.arguments!==null?event.data.arguments as Record<string,unknown>:null;
 if(typeof args?.path==='string'&&args.path)return args.path;
 if(typeof event.data.path==='string'&&event.data.path)return event.data.path;
 if(event.kind!=='tool.result')return null;
 const callId=typeof event.data.callId==='string'?event.data.callId:'';
 return callPaths.get(callId)??null;
}

// 时间线展示用：只合并紧邻的同路径 write/edit 事件；任何其他事件都会切分。
export function groupFileEdits(events:SessionEvent[]):FileEditNode[] {
 const nodes:FileEditNode[]=[],callNames=new Map<string,string>(),callPaths=new Map<string,string>(),state:{current:FileEditGroup|null}={current:null};
 const flush=()=>{
  const group=state.current;
  if(!group)return;
  state.current=null;
  const calls=group.events.filter(event=>event.kind==='tool.call').length;
  const results=group.events.filter(event=>event.kind==='tool.result').length;
  if(Math.max(calls,results)>=2)nodes.push({kind:'group',key:group.events[0].id,group});
  else for(const event of group.events)nodes.push({kind:'event',event});
 };
 for(const event of events){
  if(event.kind==='tool.call'){
   const name=event.data.name;
   if(name==='write'||name==='edit'){
    const callId=typeof event.data.id==='string'?event.data.id:'';
    const path=editPath(event,callPaths);
    if(callId){callNames.set(callId,name);if(path)callPaths.set(callId,path)}
   }
  }
  const name=editName(event,callNames),path=name?editPath(event,callPaths):null;
  if(!path){flush();nodes.push({kind:'event',event});continue}
  if(state.current?.path===path)state.current.events.push(event);
  else{flush();state.current={path,events:[event]}}
 }
 flush();
 return nodes;
}

export function summarizeFileEdits(group:FileEditGroup):FileEditSummary {
 const calls=group.events.filter(event=>event.kind==='tool.call');
 const results=group.events.filter(event=>event.kind==='tool.result');
 const names=new Set<string>();
 let add=0,del=0,hasStats=false,failed=0;
 for(const event of group.events){
  const name=event.data.name;
  if(name==='write'||name==='edit')names.add(name);
 }
 for(const result of results){
  if(result.data.isError===true)failed+=1;
  const diff=typeof result.data.diff==='string'?result.data.diff:'';
  if(!diff)continue;
  const stats=diffStats(parseUnifiedDiff(diff));
  add+=stats.add;del+=stats.del;hasStats=true;
 }
 const label:FileEditLabel=names.size===1?(names.has('write')?'写入':'编辑'):'修改';
 return {path:group.path,label,operations:Math.max(calls.length,results.length),pending:Math.max(0,calls.length-results.length),failed,add,del,hasStats,results};
}
