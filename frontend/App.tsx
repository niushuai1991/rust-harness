import {useEffect,useRef,useState,type FormEvent,type ReactNode} from 'react';
import {open,save} from '@tauri-apps/plugin-dialog';
import {ArrowUp,Check,ChevronDown,Folder,FolderOpen,GitCompareArrows,MessageSquare,Plus,Settings2,ShieldCheck,Square,Terminal,Unplug,X,Download,RefreshCw} from 'lucide-react';
import {command,desktop,errorText,send} from './api';
import {useChat} from './store';
import {ModelPanel} from './ModelPanel';
import {McpPanel} from './McpPanel';
import type {WorkspaceInfo,SessionInfo,SessionEvent,ModelConfig,ApprovalRequest,PluginInfo,FileChange} from './types';

type Panel='models'|'plugins'|'diff'|null;
const basename=(path:string)=>path.split(/[\\/]/).filter(Boolean).at(-1)??path;
const printable=(value:unknown)=>typeof value==='string'?value:JSON.stringify(value,null,2);
export default function App(){
 const [workspaces,setWorkspaces]=useState<WorkspaceInfo[]>([]),[workspaceId,setWorkspaceId]=useState('');
 const [sessions,setSessions]=useState<SessionInfo[]>([]),[sessionId,setSessionId]=useState('');
 const [models,setModels]=useState<ModelConfig[]>([]),[modelId,setModelId]=useState('');
 const [approvals,setApprovals]=useState<ApprovalRequest[]>([]),[panel,setPanel]=useState<Panel>(null);
 const [text,setText]=useState(''),[error,setError]=useState(''),[busy,setBusy]=useState(false),[running,setRunning]=useState<Record<string,boolean>>({});
 const [notice,setNotice]=useState('');
 const events=useChat(s=>s.events[sessionId]??EMPTY_EVENTS),stream=useChat(s=>s.streams[sessionId]);
 const workspace=workspaces.find(w=>w.id===workspaceId),session=sessions.find(s=>s.id===sessionId);
 const generation=useRef(0),end=useRef<HTMLDivElement>(null),activeWorkspaceId=useRef(workspaceId);
 activeWorkspaceId.current=workspaceId;
 async function attempt<T>(action:()=>Promise<T>):Promise<T|undefined>{setError('');try{return await action()}catch(e){setError(errorText(e));return undefined}}
 useEffect(()=>{if(!desktop())return;void attempt(async()=>{const [ws,ms]=await Promise.all([command<WorkspaceInfo[]>('workspace_list'),command<ModelConfig[]>('model_list')]);setWorkspaces(ws);setModels(ms);setWorkspaceId(ws[0]?.id??'');setModelId(ms[0]?.id??'')})},[]);
 useEffect(()=>{const token=++generation.current;setSessions([]);setSessionId('');if(!workspaceId)return;void attempt(async()=>{const list=await command<SessionInfo[]>('session_list',{workspaceId});if(token!==generation.current)return;setSessions(list);setSessionId(list[0]?.id??'')})},[workspaceId]);
 useEffect(()=>{if(!sessionId)return;const id=sessionId;void attempt(async()=>{const list=await command<SessionEvent[]>('session_events',{sessionId:id});useChat.getState().replace(id,list)})},[sessionId]);
 useEffect(()=>{if(!desktop())return;let alive=true;const refresh=()=>command<ApprovalRequest[]>('approval_list').then(list=>{if(alive)setApprovals(list)}).catch(e=>{if(alive)setError(errorText(e))});void refresh();const timer=setInterval(refresh,2000);return()=>{alive=false;clearInterval(timer)}},[]);
 useEffect(()=>{end.current?.scrollIntoView?.({behavior:'smooth'})},[events,stream]);
 async function openWorkspace(){setBusy(true);await attempt(async()=>{const path=await open({directory:true,multiple:false,title:'选择原项目目录'});if(!path)return;const ws=await command<WorkspaceInfo>('workspace_open',{path});setWorkspaces(old=>[...old.filter(w=>w.id!==ws.id),ws]);setWorkspaceId(ws.id);setNotice('已创建托管工作副本。原项目仅在确认同步后写入。')});setBusy(false)}
 async function newSession(){if(!workspaceId)return;const targetWorkspaceId=workspaceId,token=generation.current;await attempt(async()=>{const s=await command<SessionInfo>('session_create',{workspaceId:targetWorkspaceId,title:'新会话'});if(token!==generation.current||activeWorkspaceId.current!==targetWorkspaceId||s.workspaceId!==targetWorkspaceId)return;setSessions(old=>[s,...old]);setSessionId(s.id)})}
 async function submit(e:FormEvent){e.preventDefault();if(!sessionId||!text.trim()||running[sessionId]||!modelId)return;const id=sessionId,input=text;setText('');setRunning(old=>({...old,[id]:true}));await attempt(async()=>{await send(id,input,modelId,useChat.getState().receive)});useChat.getState().clearStream(id);try{useChat.getState().replace(id,await command<SessionEvent[]>('session_events',{sessionId:id}))}catch(e){setError(errorText(e))}setRunning(old=>({...old,[id]:false}))}
 async function cancel(){await attempt(()=>command('session_cancel',{sessionId}))}
 async function resolve(id:string,allow:boolean){await attempt(async()=>{await command('approval_resolve',{id,allow});setApprovals(old=>old.filter(a=>a.id!==id))})}
 async function exportSession(){await attempt(async()=>{const path=await save({defaultPath:'rust-harness-session.json',filters:[{name:'JSON',extensions:['json']}]});if(!path)return;await command('session_export',{sessionId,path});setNotice('会话已导出。')})}
 return <div className="app">
  <aside className="sidebar">
   <div className="brand"><span className="brand-mark"><Terminal size={23}/></span><span>Rust Harness<small>你的本地开发搭档</small></span><span className="version">α</span></div>
   <button className="workspace-picker" onClick={openWorkspace} disabled={busy||!desktop()}><FolderOpen size={19}/><span>{workspace?basename(workspace.sourcePath):'打开工作区'}<small>{workspace?'托管工作副本':'从本地项目开始'}</small></span><ChevronDown size={16}/></button>
   {workspaces.length>1&&<select aria-label="工作区" value={workspaceId} onChange={e=>setWorkspaceId(e.target.value)}>{workspaces.map(w=><option key={w.id} value={w.id}>{basename(w.sourcePath)}</option>)}</select>}
   <div className="section-label">会话 <button aria-label="新建会话" title="新建会话" onClick={newSession} disabled={!workspaceId}><Plus size={17}/></button></div>
   <nav className="sessions">{sessions.map(s=><button key={s.id} className={sessionId===s.id?'active':''} onClick={()=>{setSessionId(s.id);setText('')}}><MessageSquare size={16}/><span>{s.title}</span>{running[s.id]&&<i className="live-dot"/>}</button>)}{!sessions.length&&<p className="muted empty-small">{workspaceId?'还没有会话，点击 + 开始':'打开工作区后，会话将保存在本机。'}</p>}</nav>
   <div className="sidebar-bottom"><div className="local-badge"><i className="live-dot"/> 本地优先 · 无遥测</div><button onClick={()=>setPanel('plugins')}><Unplug size={17}/>插件与权限</button><button onClick={()=>setPanel('models')}><Settings2 size={17}/>模型设置 <span className="count">{models.length}</span></button></div>
  </aside>
  <main className="main"><header className="topbar"><div className="breadcrumb"><Folder size={15}/>{workspace?basename(workspace.sourcePath):'工作台'}<span>/</span><strong>{session?.title??'新的可能，从这里开始'}</strong></div><div className="toolbar"><button className="icon-button" title="导出会话" aria-label="导出会话" disabled={!sessionId} onClick={exportSession}><Download size={17}/></button><button className="soft" onClick={()=>setPanel('diff')} disabled={!workspaceId}><GitCompareArrows size={16}/>查看更改</button></div></header>
   {!desktop()&&<div className="host-banner"><Terminal size={16}/>当前为界面预览，需要桌面宿主才能访问工作区和模型。</div>}
   {error&&<div role="alert" className="alert"><span>{error}</span><button aria-label="关闭错误" onClick={()=>setError('')}><X size={16}/></button></div>}
   {notice&&<div role="status" className="notice"><Check size={15}/>{notice}<button aria-label="关闭提示" onClick={()=>setNotice('')}><X size={15}/></button></div>}
   <section className="conversation" aria-label="聊天消息">
    {!events.length&&!stream&&<div className="welcome"><span className="welcome-icon"><Terminal size={35}/></span><div className="eyebrow">BUILD WITH CONFIDENCE</div><h1>想把什么变成现实？</h1><p>理解代码、探索想法，或完成下一次改进。<br/>在独立的工作副本中协作，每一处更改都由你掌控。</p><div className="suggestions">{['梳理项目结构和关键模块','检查代码，找出值得改进的地方','帮我规划并实现一个新功能'].map((s,i)=><button key={s} onClick={()=>setText(s)} disabled={!sessionId}><span>0{i+1}</span>{s}<ArrowUp size={14}/></button>)}</div><div className="welcome-foot"><ShieldCheck size={15}/>工作副本隔离 · 操作可审批 · 更改可预览</div></div>}
    <div className="messages">{events.map(event=><EventCard key={event.id} event={event}/>)}{stream&&<article className="message assistant"><div className="avatar">R</div><div className="message-body"><div className="message-label">Rust Harness <span className="stream-label">正在思考与执行</span></div>{stream.reasoning&&<details className="reasoning"><summary>思考过程</summary><pre>{stream.reasoning}</pre></details>}<div className="prose">{stream.text||'正在处理…'}<span className="cursor"/></div></div></article>}<div ref={end}/></div>
   </section>
   <footer className="composer-area"><form className="composer" onSubmit={submit}><textarea aria-label="消息" placeholder={sessionId?'描述任务，或继续你的想法…':'打开工作区并新建会话，开始协作'} value={text} onChange={e=>setText(e.target.value)} disabled={!sessionId} onKeyDown={e=>{if(e.key==='Enter'&&!e.shiftKey&&!e.nativeEvent.isComposing){e.preventDefault();e.currentTarget.form?.requestSubmit()}}}/><div className="composer-bottom"><div className="model-select"><span className="model-dot"/><select aria-label="当前模型" value={modelId} onChange={e=>setModelId(e.target.value)}><option value="" disabled>选择模型</option>{models.map(m=><option value={m.id} key={m.id}>{m.name}</option>)}</select></div><div className="send-controls"><span>Enter 发送 · Shift + Enter 换行</span>{running[sessionId]?<button type="button" className="send" aria-label="取消生成" onClick={cancel}><Square size={16}/></button>:<button className="send" type="submit" aria-label="发送消息" disabled={!text.trim()||!sessionId||!modelId}><ArrowUp size={19}/></button>}</div></div></form><div className="footer-note">原项目保持安全。模型生成的更改需要你确认后才会同步。</div></footer>
  </main>
  {panel==='models'&&<ModelPanel models={models} onClose={()=>setPanel(null)} onSaved={async()=>{const ms=await command<ModelConfig[]>('model_list');setModels(ms);if(!ms.some(m=>m.id===modelId))setModelId(ms[0]?.id??'')}}/>}
  {panel==='plugins'&&<PluginPanel workspaceId={workspaceId} onClose={()=>setPanel(null)}/>}
  {panel==='diff'&&<DiffPanel key={workspaceId} workspaceId={workspaceId} onClose={()=>setPanel(null)}/>}
  {approvals[0]&&<Modal title="操作需要你的批准" onClose={()=>void resolve(approvals[0].id,false)}><div className="approval-heading"><ShieldCheck size={24}/><strong>{approvals[0].tool}</strong></div><p>{approvals[0].reason}</p><pre className="code-block">{printable(approvals[0].arguments)}</pre><p className="muted">会话：{sessions.find(s=>s.id===approvals[0].sessionId)?.title??approvals[0].sessionId}</p><div className="actions"><button onClick={()=>resolve(approvals[0].id,false)}>拒绝</button><button className="primary" onClick={()=>resolve(approvals[0].id,true)}>批准此次操作</button></div></Modal>}
 </div>
}
const EMPTY_EVENTS:SessionEvent[]=[];
export function EventCard({event}:{event:SessionEvent}) {
 const {kind,data}=event;
 if(kind==='user.message')return <article className="message user"><div className="avatar">你</div><div className="message-body"><div className="message-label">你</div><div className="prose">{printable(data.content)}</div></div></article>;
 if(kind==='assistant.message'||kind==='assistant.attempt')return <article className="message assistant"><div className="avatar">R</div><div className="message-body"><div className="message-label">Rust Harness</div>{!!data.reasoning&&<details className="reasoning"><summary>思考过程</summary><pre>{printable(data.reasoning)}</pre></details>}<div className="prose">{printable(data.content)}</div>{!!data.error&&<div className="inline-error">{printable(data.error)}</div>}</div></article>;
 if(kind==='tool.call'||kind==='tool.result')return <details className="tool-card"><summary><Terminal size={15}/><strong>{printable(data.name)}</strong><span>{kind==='tool.call'?'工具调用':data.isError?'执行失败':'已完成'}</span></summary><pre>{printable(data.arguments??data.content)}</pre></details>;
 if(kind==='compaction.summary')return <details className="tool-card"><summary>上下文摘要</summary><pre>{printable(data.content)}</pre></details>;
 if(kind==='turn.end'&&data.status&&data.status!=='completed')return <div className="turn-status">本轮状态：{printable(data.status)}</div>;
 return null;
}
export function Modal({title,onClose,children,wide=false}:{title:string;onClose:()=>void;children:ReactNode;wide?:boolean}) {
 const dialog=useRef<HTMLElement>(null);
 useEffect(()=>{const previous=document.activeElement as HTMLElement|null;dialog.current?.focus();return()=>previous?.focus()},[]);
 const trap=(e:React.KeyboardEvent<HTMLElement>)=>{
  if(e.key==='Escape'){e.stopPropagation();onClose();return}
  if(e.key!=='Tab')return;
  const elements=[...(dialog.current?.querySelectorAll<HTMLElement>('button:not(:disabled),input:not(:disabled),select:not(:disabled),textarea:not(:disabled),[tabindex="0"]')??[])];
  const first=elements[0],last=elements.at(-1);
  if(!first){e.preventDefault();return}
  if(e.shiftKey&&(document.activeElement===first||document.activeElement===dialog.current)){e.preventDefault();last?.focus()}
  else if(!e.shiftKey&&(document.activeElement===last||document.activeElement===dialog.current)){e.preventDefault();first.focus()}
 };
 return <div className="overlay" onMouseDown={e=>{if(e.target===e.currentTarget)onClose()}}><section ref={dialog} tabIndex={-1} role="dialog" aria-modal="true" aria-label={title} className={`modal ${wide?'wide':''}`} onKeyDown={trap}><header><h2>{title}</h2><button aria-label="关闭面板" onClick={onClose}><X size={19}/></button></header><div className="modal-content">{children}</div></section></div>}
function PluginPanel({workspaceId,onClose}:{workspaceId:string;onClose:()=>void}) {
 const [plugins,setPlugins]=useState<PluginInfo[]>([]),[path,setPath]=useState(''),[permissions,setPermissions]=useState(''),[error,setError]=useState(''),[busy,setBusy]=useState(false),[removeId,setRemoveId]=useState('');
 const load=async()=>setPlugins(await command<PluginInfo[]>('plugin_list'));
 useEffect(()=>{void load().catch(e=>setError(errorText(e)))},[]);
 async function action(fn:()=>Promise<unknown>){setBusy(true);setError('');try{await fn();await load()}catch(e){setError(errorText(e))}finally{setBusy(false)}}
 return <Modal title="插件与权限" onClose={onClose}><p className="muted">从本地导入 WASM 插件。插件仅获得声明、授权与工作区策略的交集权限。</p>{error&&<p role="alert" className="inline-error">{error}</p>}<form className="settings-form" onSubmit={e=>{e.preventDefault();void action(()=>command('plugin_install',{path,permissions:permissions.split(',').map(s=>s.trim()).filter(Boolean)}))}}><label>插件目录或 ZIP 路径<div className="input-row"><input required value={path} onChange={e=>setPath(e.target.value)} placeholder="输入插件完整路径"/><button type="button" disabled={!desktop()} onClick={async()=>{try{const result=await open({multiple:false,directory:true});if(result)setPath(result)}catch(e){setError(errorText(e))}}}>浏览目录</button></div></label><label>授予权限（逗号分隔，默认无权限）<input value={permissions} onChange={e=>setPermissions(e.target.value)} placeholder="按插件清单填写能力名称"/></label><button className="primary" disabled={busy||!desktop()}>导入并授权</button></form><div className="plugin-list">{!plugins.length&&<p className="empty-small muted">暂无已安装插件。</p>}{plugins.map(p=><article key={p.id}><div className="plugin-heading"><Unplug size={19}/><strong>{p.name}</strong><small>{p.version} · {p.kind}</small></div><p>{p.description}</p><div className="permission-tags">{p.permissions.length?p.permissions.map(x=><code key={x}>{x}</code>):<span>无额外授权</span>}</div><div className="plugin-actions"><small>{p.status}</small><button disabled={busy} onClick={()=>action(()=>command('plugin_enable',{id:p.id,enabled:!p.enabled}))}>{p.enabled?'停用':'启用'}</button><button className="danger" disabled={busy} onClick={()=>setRemoveId(p.id)}>移除</button></div>{removeId===p.id&&<div className="confirm">确定移除此插件？<button onClick={()=>setRemoveId('')}>取消</button><button className="danger" onClick={()=>action(async()=>{await command('plugin_remove',{id:p.id});setRemoveId('')})}>确认移除</button></div>}</article>)}</div><McpPanel workspaceId={workspaceId}/></Modal>
}
export function DiffPanel({workspaceId,onClose}:{workspaceId:string;onClose:()=>void}) {
 const [changes,setChanges]=useState<FileChange[]>([]),[selected,setSelected]=useState<string[]>([]),[error,setError]=useState(''),[busy,setBusy]=useState(false),[confirm,setConfirm]=useState(false),[loaded,setLoaded]=useState(false),[status,setStatus]=useState('');
 async function load(){setBusy(true);setConfirm(false);setError('');try{const list=await command<FileChange[]>('workspace_diff',{workspaceId});setChanges(list);setSelected(list.filter(c=>!c.conflict).map(c=>c.path));setLoaded(true)}catch(e){setError(errorText(e))}finally{setBusy(false)}}
 useEffect(()=>{void load()},[workspaceId]);
 async function apply(){setBusy(true);setError('');try{await command('workspace_apply',{workspaceId,paths:selected,reviewed:changes.filter(change=>selected.includes(change.path))});setConfirm(false);setStatus('已将选中更改同步到原项目。');await load()}catch(e){setError(errorText(e));setConfirm(false)}finally{setBusy(false)}}
 return <Modal wide title="审阅与同步更改" onClose={onClose}><div className="diff-intro"><p className="muted">对比托管工作副本与原项目。勾选要同步的文件，确认后写入原项目；冲突文件需先处理。本地 stdio MCP 长期连接会占用副本，同步前请在插件面板断开。</p><button disabled={busy} onClick={load}><RefreshCw size={15}/>刷新</button></div>{error&&<p role="alert" className="inline-error">{error}</p>}{status&&<p role="status" className="success">{status}</p>}{loaded&&!changes.length&&<div className="diff-empty"><Check size={28}/><h3>工作副本没有待同步更改</h3><p className="muted">下一次改进，从一个想法开始。</p></div>}{changes.map(c=><article className="diff-file" key={c.path}><div className="diff-file-heading"><label><input type="checkbox" checked={selected.includes(c.path)} disabled={c.conflict||busy} onChange={e=>setSelected(old=>e.target.checked?[...old,c.path]:old.filter(p=>p!==c.path))}/><strong>{c.path}</strong></label><span className={c.conflict?'conflict':'change-kind'}>{c.conflict?'原项目已变更 · 冲突':c.kind}</span></div><div className="diff-columns"><div><small>原内容</small><pre>{c.beforeText??'二进制文件或无原内容'}</pre></div><div><small>工作副本</small><pre>{c.afterText??'二进制文件或已删除'}</pre></div></div></article>)}<div className="actions"><span className="muted">已选择 {selected.length} 个文件</span>{confirm?<><span>确认写入原项目？</span><button onClick={()=>setConfirm(false)} disabled={busy}>返回审阅</button><button className="primary" onClick={apply} disabled={busy}>{busy?'同步中…':'确认应用更改'}</button></>:<button className="primary" disabled={busy||!selected.length} onClick={()=>setConfirm(true)}>同步选中文件</button>}</div></Modal>
}



