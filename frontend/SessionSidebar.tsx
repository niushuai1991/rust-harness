import {Fragment} from 'react';
import {Archive,ChevronRight,FolderOpen,MessageSquare,Pin,Plus,Search,Settings,Target,Timer} from 'lucide-react';
import {desktop} from './api';
import {GROUP_LABELS,groupByProject,groupByTime,type ViewMode} from './sessionList';
import {useSidePanel} from './sidePanelStore';
import type {SessionInfo,WorkspaceInfo} from './types';

type SidebarProps={
 sidebarWidth:number; newSession:()=>void; sessionCreating:boolean;
 query:string; setQuery:(value:string)=>void; viewMode:ViewMode; chooseView:(value:ViewMode)=>void;
 dismissSessionPop:()=>void; visibleSessions:SessionInfo[]; workspaces:WorkspaceInfo[]; sessionId:string;
 openFromSidebar:(s:SessionInfo)=>void; showSessionContextMenu:(event:React.MouseEvent<HTMLButtonElement>,s:SessionInfo)=>void;
 openSessionPop:(s:SessionInfo,node:HTMLButtonElement)=>void; scheduleSessionPopClose:()=>void;
 pendingAskIds:Set<string>; projectName:(id:string)=>string; running:Record<string,boolean>; activeIds:Set<string>;
 archiveSession:(s:SessionInfo)=>void; visibleWorkspaces:WorkspaceInfo[]; collapsed:string[];
 setWsContextMenu:(value:{workspaceId:string;x:number;y:number})=>void; toggleCollapsed:(id:string)=>void;
 duplicateSourcePaths:Set<string>; newSessionIn:(id:string)=>Promise<void>; togglePinned:(ws:WorkspaceInfo)=>Promise<void>;
 workspaceId:string; openWorkspace:()=>void; busy:boolean; setPanel:(panel:'automations')=>void;
 automationCount:number|null; openSettings:()=>void; startSidebarDrag:(event:React.PointerEvent<HTMLDivElement>)=>void;
};

export function SessionSidebar({sidebarWidth,newSession,sessionCreating,query,setQuery,viewMode,chooseView,dismissSessionPop,visibleSessions,workspaces,sessionId,openFromSidebar,showSessionContextMenu,openSessionPop,scheduleSessionPopClose,pendingAskIds,projectName,running,activeIds,archiveSession,visibleWorkspaces,collapsed,setWsContextMenu,toggleCollapsed,duplicateSourcePaths,newSessionIn,togglePinned,workspaceId,openWorkspace,busy,setPanel,automationCount,openSettings,startSidebarDrag}:SidebarProps){
 return <aside className="sidebar" aria-label="项目列表" style={{width:sidebarWidth}}>
   <button className="new-chat" title="新建聊天" onClick={newSession} disabled={sessionCreating}><Plus size={16}/><span>新建聊天</span></button>
   <div className="session-search"><Search size={14} aria-hidden="true"/><input value={query} onChange={e=>setQuery(e.target.value)} placeholder="搜索会话…" aria-label="搜索会话"/></div>
   <div className="section-label"><span>项目列表</span><span className="label-spacer"/><button type="button" role="switch" aria-checked={viewMode==='project'} aria-label="切换视图：项目 / 最近" className="view-switch" onClick={()=>chooseView(viewMode==='recent'?'project':'recent')}><span className="knob" aria-hidden="true"/><span className="sw-label a" aria-hidden="true">项目</span><span className="sw-label b" aria-hidden="true">最近</span></button></div>
   <nav className="sessions session-scrollbar" aria-label="会话列表" onScrollCapture={dismissSessionPop}>
    {viewMode==='recent'?groupByTime(visibleSessions,new Date()).map(group=><Fragment key={group.group}>
     <div className="group-head">{GROUP_LABELS[group.group]}</div>
     {group.sessions.map(s=>{const ws=workspaces.find(w=>w.id===s.workspaceId);return <button key={s.id} className={`session-row${sessionId===s.id?' active':''}`} onClick={()=>openFromSidebar(s)} onContextMenu={event=>showSessionContextMenu(event,s)} onMouseEnter={event=>openSessionPop(s,event.currentTarget)} onMouseLeave={scheduleSessionPopClose}><MessageSquare size={16}/><span className="session-lines"><span className="session-l1"><span className="session-title">{s.title}</span>{pendingAskIds.has(s.id)&&<em className="ask-badge">待答复</em>}<span role="button" tabIndex={0} className="session-archive" aria-label="存档会话" title="存档会话" onClick={event=>{event.stopPropagation();archiveSession(s)}}><Archive size={13}/></span></span><span className="session-l2">{(ws||s.workspaceId==='')&&<span>{projectName(s.workspaceId)}</span>}{ws?.pinned&&<Pin size={10} aria-hidden="true"/>}{(running[s.id]||activeIds.has(s.id))&&<i className="live-dot" aria-label="运行中"/>}</span></span></button>})}
    </Fragment>):groupByProject(visibleSessions,visibleWorkspaces,!query).map(({workspace:ws,sessions:list})=>{
     const open=!collapsed.includes(ws.id);
     return <div key={ws.id} className="ws-group">
      <div className="ws-head" onContextMenu={event=>{if(ws.id==='')return;event.preventDefault();setWsContextMenu({workspaceId:ws.id,x:Math.max(4,Math.min(event.clientX,window.innerWidth-164)),y:Math.max(4,Math.min(event.clientY,window.innerHeight-216))})}}>
       <button type="button" className="ws-toggle" aria-expanded={open} onClick={()=>toggleCollapsed(ws.id)}><ChevronRight size={13} className="chev" aria-hidden="true"/><span className="ws-name">{ws.name}</span>{duplicateSourcePaths.has(ws.sourcePath)&&<em className="dup-badge" title="同一源目录存在多条项目记录">重复</em>}<span className="ws-count">{list.length}</span></button>
       <button type="button" className="ws-add" aria-label={`在 ${ws.name} 新建聊天`} title="新建聊天" disabled={sessionCreating} onClick={()=>void newSessionIn(ws.id)}><Plus size={13}/></button>
       {ws.id!==''&&       <button type="button" className={`ws-pin${ws.pinned?' on':''}`} aria-label={`${ws.pinned?'取消置顶':'置顶'} ${ws.name}`} title={ws.pinned?'取消置顶':'置顶项目'} onClick={()=>void togglePinned(ws)}><Pin size={13}/></button>}
      </div>
      {open&&list.map(s=><button key={s.id} className={`session-row one${sessionId===s.id?' active':''}`} onClick={()=>openFromSidebar(s)} onContextMenu={event=>showSessionContextMenu(event,s)} onMouseEnter={event=>openSessionPop(s,event.currentTarget)} onMouseLeave={scheduleSessionPopClose}><MessageSquare size={16}/><span className="session-title">{s.title}</span>{pendingAskIds.has(s.id)&&<em className="ask-badge">待答复</em>}{(running[s.id]||activeIds.has(s.id))&&<i className="live-dot" aria-label="运行中"/>}<span role="button" tabIndex={0} className="session-archive" aria-label="存档会话" title="存档会话" onClick={event=>{event.stopPropagation();archiveSession(s)}}><Archive size={13}/></span></button>)}
     </div>;
    })}
    {(!visibleSessions.length&&!(viewMode==='project'&&!query&&workspaces.length))&&<p className="muted empty-small">{query?'无匹配会话。':workspaceId?'还没有会话，点击「新建聊天」开始':'打开工作区后，会话将保存在本机。'}</p>}
    {!visibleSessions.length&&!query&&!workspaceId&&<button className="empty-open" onClick={openWorkspace} disabled={busy||!desktop()}><FolderOpen size={14}/>打开工作区</button>}
   </nav>
   <div className="sidebar-bottom"><button onClick={()=>useSidePanel.getState().openPlans()}><Target size={17}/>计划与目标</button><button onClick={()=>setPanel('automations')}><Timer size={17}/>自动化{automationCount!==null&&<span className="count">{automationCount}</span>}</button><button aria-label="设置" onClick={()=>openSettings()}><Settings size={17}/>设置</button></div>
   <div className="sidebar-resizer" role="separator" aria-orientation="vertical" aria-label="调整列表宽度" onPointerDown={startSidebarDrag}/>
  </aside>;
}
