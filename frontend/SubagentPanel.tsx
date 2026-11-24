import {useEffect,useRef} from 'react';
import {Bot,X} from 'lucide-react';
import type {SessionInfo} from './types';

export type SubagentPanelProps={
 sessions:SessionInfo[];
 activeIds:Set<string>;
 // 侧聊会话：点击在右侧抽屉打开，行内显示「侧聊」标记；普通子代理仍切换主会话。
 sideIds?:Set<string>;
 onOpen:(session:SessionInfo)=>void;
 onCancel:(sessionId:string)=>void;
 onClose:()=>void;
};

/// 输入框下方 Bot 图标弹出的子代理面板；运行中排前面，点击行跳转会话，运行中的可停止。
/// 数据由 App 侧过滤传入（当前会话的子代理会话），运行状态复用既有的 activeIds 轮询。
export function SubagentPanel({sessions,activeIds,sideIds,onOpen,onCancel,onClose}:SubagentPanelProps) {
 const root=useRef<HTMLElement>(null);
 useEffect(()=>{
  // 触发按钮的收起由 App 的 toggle 负责，这里只处理点外部与 Escape。
  function outside(e:MouseEvent){
   const t=e.target as Node;
   if(root.current?.contains(t))return;
   if(t instanceof Element&&t.closest('[aria-controls="subagent-popover"]'))return;
   onClose();
  }
  function key(e:KeyboardEvent){if(e.key==='Escape')onClose()}
  document.addEventListener('mousedown',outside);
  document.addEventListener('keydown',key);
  return()=>{document.removeEventListener('mousedown',outside);document.removeEventListener('keydown',key)};
 },[onClose]);
 const ordered=[...sessions].sort((a,b)=>{
  const ra=activeIds.has(a.id)?1:0,rb=activeIds.has(b.id)?1:0;
  if(ra!==rb)return rb-ra;
  return Date.parse(b.updatedAt)-Date.parse(a.updatedAt);
 });
 return <section ref={root} id="subagent-popover" className="subagent-panel" role="region" aria-label="子代理">
  <header className="subagent-panel-head">
   <strong><Bot size={14} aria-hidden="true"/>子代理</strong>
   <button type="button" className="icon-button" aria-label="收起子代理面板" onClick={onClose}><X size={14}/></button>
  </header>
  {ordered.length?<div className="subagent-panel-list">
   {ordered.map(s=>{
    const live=activeIds.has(s.id),side=sideIds?.has(s.id)??false;
    return <div key={s.id} className="subagent-item">
     <button type="button" className="subagent-open" title={s.title} onClick={()=>{onOpen(s);onClose()}}>
      <span className="session-title">{s.title}</span>
      {side&&<em className="side-chat-badge">侧聊</em>}
      {live&&<i className="live-dot" aria-label="子代理运行中"/>}
     </button>
     {live&&<button type="button" className="subagent-stop" aria-label="停止子代理" title={`停止 ${s.title}`} onClick={()=>onCancel(s.id)}><X size={13}/></button>}
    </div>;
   })}
  </div>:<p className="muted empty-small">暂无子代理会话。<br/>模型创建的子代理任务会显示在这里。</p>}
 </section>;
}
