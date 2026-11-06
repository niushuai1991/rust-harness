import {useEffect,useState} from 'react';
import {ListTodo,MessageSquare,RefreshCw,Target} from 'lucide-react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import {command,errorText} from './api';
import {Modal,TodoList} from './App';
import type {PlanStateOverview,SessionEvent} from './types';

const STATE_KINDS=['todo.state','plan.state','goal.state'];
const KIND_LABEL:{[kind:string]:string}={'todo.state':'待办清单','plan.state':'计划','goal.state':'目标'};
const localTime=(iso:string)=>{const t=Date.parse(iso);return Number.isFinite(t)?new Date(t).toLocaleString():iso};
const todoItems=(data:Record<string,unknown>)=>Array.isArray(data.items)?(data.items as Record<string,unknown>[]):[];
const isDone=(item:Record<string,unknown>)=>item.done===true||item.completed===true||item.status==='completed';
const todoSummary=(data:Record<string,unknown>)=>{const items=todoItems(data);return `${items.length} 项 · ${items.filter(isDone).length} 已完成`};
const planSnippet=(data:Record<string,unknown>)=>{const text=String(data.content??'');return text.length>80?`${text.slice(0,80)}…`:text};

export function PlansPanel({currentSessionId,onOpenSession,onClose}:{currentSessionId:string;onOpenSession:(sessionId:string,workspaceId:string)=>void;onClose:()=>void}) {
 const [entries,setEntries]=useState<PlanStateOverview[]>([]),[error,setError]=useState(''),[loaded,setLoaded]=useState(false);
 const [expanded,setExpanded]=useState(''),[history,setHistory]=useState<Record<string,SessionEvent[]|null>>({});
 async function load(){setError('');try{setEntries(await command<PlanStateOverview[]>('plan_overview'));setLoaded(true)}catch(e){setError(errorText(e))}}
 useEffect(()=>{void load()},[]);
 const order=entries.map(e=>e.sessionId).filter((id,index,all)=>all.indexOf(id)===index);
 const groups=order.map(sessionId=>({sessionId,items:entries.filter(e=>e.sessionId===sessionId)}));
 async function toggleHistory(sessionId:string){
  if(expanded===sessionId){setExpanded('');return}
  setExpanded(sessionId);
  if(history[sessionId]===undefined){
   setHistory(old=>({...old,[sessionId]:null}));
   try{
    const list=await command<SessionEvent[]>('session_events',{sessionId});
    setHistory(old=>({...old,[sessionId]:list.filter(e=>STATE_KINDS.includes(e.kind)).reverse()}));
   }catch(e){setError(errorText(e));setHistory(old=>{const next={...old};delete next[sessionId];return next})}
  }
 }
 return <Modal wide title="计划与目标" onClose={onClose}>
  <div className="diff-intro"><p className="muted">所有会话的最新待办、计划与目标，由模型经工具写入，这里只读查看；事件流仍是事实来源。</p><button disabled={!loaded} onClick={load}><RefreshCw size={15}/>刷新</button></div>
  {error&&<p role="alert" className="inline-error">{error}</p>}
  {loaded&&!groups.length&&<div className="diff-empty"><Target size={28}/><h3>还没有任何计划或目标</h3><p className="muted">会话中让模型记录待办、计划或目标后，会在这里汇总。</p></div>}
  <div className="plan-groups">{groups.map(g=>
   <article key={g.sessionId} className="plan-group">
    <header className="plan-group-heading"><MessageSquare size={16}/><strong>{g.items[0].sessionTitle}</strong><small className="muted">更新于 {localTime(g.items[0].updatedAt)}</small>
     <div className="plan-group-actions">
      <button type="button" onClick={()=>onOpenSession(g.sessionId,g.items[0].workspaceId)}>{g.sessionId===currentSessionId?'当前会话':'查看会话'}</button>
      <button type="button" onClick={()=>void toggleHistory(g.sessionId)}>{expanded===g.sessionId?'收起历史':'演变历史'}</button>
     </div>
    </header>
    {expanded===g.sessionId&&(history[g.sessionId]===null?<p className="muted">加载中…</p>:
     <div className="plan-history">{!history[g.sessionId]?.length&&<p className="muted">该会话暂无状态记录。</p>}
      {history[g.sessionId]?.map(h=><div key={h.id} className="plan-history-row"><small className="muted">{localTime(h.createdAt)} · {KIND_LABEL[h.kind]??h.kind}</small><span>{h.kind==='todo.state'?todoSummary(h.data):planSnippet(h.data)}</span></div>)}
     </div>)}
    {g.items.map(e=>
     <section key={e.kind} className="plan-entry">
      <div className="subagent-row">{e.kind==='todo.state'?<ListTodo size={15}/>:<Target size={15}/>}<strong>{KIND_LABEL[e.kind]??e.kind}</strong><small className="muted">{localTime(e.updatedAt)}</small></div>
      {e.kind==='todo.state'?<TodoList items={todoItems(e.data)}/>:<div className="prose"><ReactMarkdown remarkPlugins={[remarkGfm]}>{String(e.data.content??'')}</ReactMarkdown></div>}
     </section>)}
   </article>)}
  </div>
 </Modal>;
}
