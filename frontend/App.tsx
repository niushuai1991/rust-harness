import {Fragment,useEffect,useMemo,useRef,useState,type FormEvent,type ReactNode} from 'react';
import {open,save} from '@tauri-apps/plugin-dialog';
import {ArrowUp,Bot,Bookmark,CalendarClock,Check,ChevronDown,ChevronRight,FileCode,Folder,FolderOpen,GitCompareArrows,ListTodo,MessageSquare,Paperclip,Pencil,Pin,Plus,Search,Send,Settings2,ShieldCheck,Square,Sparkles,Target,Terminal,Timer,Unplug,X,Download} from 'lucide-react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import {command,desktop,errorText,send} from './api';
import {KNOWN_APPROVALS} from './types';
import {INIT_PROMPT,SLASH_COMMANDS,commandTokenUnderCursor,expandSlashPrompt,filterSlashCommands,type SlashSpec} from './slashCommands';
import {filterSkillRows,skillRowsFrom,skillTokenUnderCursor,type SkillRow,type SkillToken} from './skills';
import {GROUP_LABELS,filterByTitle,groupByProject,groupByTime,relativeLabel,type ViewMode} from './sessionList';
import {useChat} from './store';
import {ModelPanel} from './ModelPanel';
import ModelSelect from './ModelSelect';
import {UsageRing} from './UsageRing';
import {PresetsPanel} from './PresetsPanel';
import {SkillsPanel} from './SkillsPanel';
import {McpPanel} from './McpPanel';
import {PluginContributionsPanel,PluginThemeHost} from './PluginContributionsPanel';
import {PluginResultCards} from './PluginResultCards';
import {BackgroundTasksPanel,JOB_STATUS,SCHEDULE_STATUS} from './BackgroundTasksPanel';
import {PlansPanel} from './PlansPanel';
import {AutomationsPanel} from './AutomationsPanel';
import {SidePanel} from './SidePanel';
import {useSidePanel} from './sidePanelStore';
import type {WorkspaceInfo,SessionInfo,SessionEvent,ModelConfig,ProviderInfo,DefaultSelection,ApprovalRequest,ApprovalPolicy,PluginInfo,ArtifactMetadata,PresetInfo,SkillInfo} from './types';

type Panel='models'|'presets'|'skills'|'plugins'|'tasks'|'plans'|'automations'|null;
const TERMINAL_STATUS:{[status:string]:string}={running:'运行中',exited:'已退出',terminated:'已终止',interrupted:'已中断'};
// 桌面权限模式（与 contracts 五档一一对应）；改动仍只落隔离工作副本，进原项目仍需 apply 确认。
const APPROVAL_MODES:{value:ApprovalPolicy;label:string;description:string}[]=[
 {value:'ask',label:'每次确认',description:'每个工具调用前逐一确认'},
 {value:'edit',label:'自动编辑',description:'写文件自动放行，其余仍确认'},
 {value:'guarded',label:'自主',description:'自动执行，危险命令仍需确认'},
 {value:'auto',label:'完全访问',description:'全部自动放行（改动仍只落工作副本）'},
 {value:'never',label:'只读',description:'需审批的工具一律拒绝'},
];
const PLAN_USAGE='用法：/plan <任务描述>，发送后模型制定实施计划并经 plan_write 记录。';
const GOAL_USAGE='用法：/goal <目标描述>，发送后模型经 goal_write 记录本会话目标。';
const basename=(path:string)=>path.split(/[\\/]/).filter(Boolean).at(-1)??path;
const printable=(value:unknown)=>typeof value==='string'?value:JSON.stringify(value,null,2);
export default function App(){
 const [workspaces,setWorkspaces]=useState<WorkspaceInfo[]>([]),[workspaceId,setWorkspaceId]=useState('');
 const [sessions,setSessions]=useState<SessionInfo[]>([]),[sessionId,setSessionId]=useState(''),[sessionBusy,setSessionBusy]=useState(false);
 // 跨工作区全量会话（侧栏「最近 / 项目」视图数据源）；sessions 始终为当前工作区子集。
 const [allSessions,setAllSessions]=useState<SessionInfo[]>([]);
 const [models,setModels]=useState<ModelConfig[]>([]),[modelId,setModelId]=useState(''),[providers,setProviders]=useState<ProviderInfo[]>([]),[effort,setEffort]=useState<string|null>(null);
 const [presets,setPresets]=useState<PresetInfo[]>([]),[presetId,setPresetId]=useState('');
 const [approvals,setApprovals]=useState<ApprovalRequest[]>([]),[panel,setPanel]=useState<Panel>(null);
 // 权限模式：默认档全局持久化；会话内切档记录显式覆盖（后端同口径：覆盖优先于默认档）。
 const [approvalMode,setApprovalMode]=useState<ApprovalPolicy>('ask'),[approvalOverrides,setApprovalOverrides]=useState<Record<string,ApprovalPolicy>>({});
 const [automationCount,setAutomationCount]=useState<number|null>(null);
 const [skills,setSkills]=useState<SkillInfo[]>([]);
 const sideOpen=useSidePanel(s=>s.open);
 const [activeIds,setActiveIds]=useState<Set<string>>(()=>new Set());
 const [text,setText]=useState(''),[error,setError]=useState(''),[busy,setBusy]=useState(false),[running,setRunning]=useState<Record<string,boolean>>({});
 const [cursor,setCursor]=useState(0),[slashIndex,setSlashIndex]=useState(0),[dismissedToken,setDismissedToken]=useState('');
 const textareaRef=useRef<HTMLTextAreaElement>(null);
 // 补全弹层（/ 与 $）当前选中项：方向键移动选中项时滚动跟随用。
 const selectedOptionRef=useRef<HTMLButtonElement|null>(null);
 const [notice,setNotice]=useState('');
 // 会话列表「最近 / 项目」视图：视图模式与项目折叠状态存 localStorage，跨重启保留。
 const [viewMode,setViewMode]=useState<ViewMode>(()=>{try{return localStorage.getItem('sessionView')==='project'?'project':'recent'}catch{return 'recent'}});
 const [collapsed,setCollapsed]=useState<string[]>(()=>{try{const v=JSON.parse(localStorage.getItem('sessionCollapsed')??'[]');return Array.isArray(v)?v.filter(x=>typeof x==='string'):[]}catch{return []}});
 const [query,setQuery]=useState('');
 const [pluginRevision,setPluginRevision]=useState(0);
 const [attachments,setAttachments]=useState<ArtifactMetadata[]>([]),[importing,setImporting]=useState(false);
 const [plusOpen,setPlusOpen]=useState(false);
 // 工作区命名弹窗：create=打开工作区时命名（留空用目录名），rename=项目重命名。
 const [wsDialog,setWsDialog]=useState<{mode:'create';path:string;name:string}|{mode:'rename';id:string;name:string}|null>(null);
 // 模型浮层卡片与切换标记（仅内存：切换模型在会话流里的临时提示，不落库）。
 const [modelMenuOpen,setModelMenuOpen]=useState(false);
 const [modelNotes,setModelNotes]=useState<{sessionId:string;text:string}[]>([]);
 useEffect(()=>{if(!plusOpen)return;const close=(e:MouseEvent)=>{if(!(e.target as Element|null)?.closest?.('.composer-plus-wrap'))setPlusOpen(false)};const esc=(e:KeyboardEvent)=>{if(e.key==='Escape')setPlusOpen(false)};document.addEventListener('mousedown',close);document.addEventListener('keydown',esc);return()=>{document.removeEventListener('mousedown',close);document.removeEventListener('keydown',esc)}},[plusOpen]);
 const attachmentEpoch=useRef(0),activeSessionId=useRef(sessionId);activeSessionId.current=sessionId;
 const pendingJump=useRef('');
 useEffect(()=>{attachmentEpoch.current++;setAttachments([]);setImporting(false)},[sessionId,workspaceId]);
 // 切换工作区时重置右侧面板（预览路径与视图归属工作区，避免串状态）。
 useEffect(()=>{useSidePanel.getState().reset()},[workspaceId]);
 // 侧栏「自动化」徽标：跟随工作区拉取任务数，面板增删后经 onChanged 刷新。
 useEffect(()=>{if(!workspaceId||!desktop()){setAutomationCount(null);return}let alive=true;command<unknown[]>('automation_list',{workspaceId}).then(list=>{if(alive)setAutomationCount(list.length)}).catch(()=>{if(alive)setAutomationCount(null)});return()=>{alive=false}},[workspaceId]);
 // 技能名单：跟随工作区拉取（composer $ 补全候选）；读取失败退化为空（无补全）。
 useEffect(()=>{if(!workspaceId||!desktop()){setSkills([]);return}let alive=true;command<SkillInfo[]>('skill_list',{workspaceId}).then(list=>{if(alive)setSkills(Array.isArray(list)?list:[])}).catch(()=>{if(alive)setSkills([])});return()=>{alive=false}},[workspaceId]);
 const events=useChat(s=>s.events[sessionId]??EMPTY_EVENTS),stream=useChat(s=>s.streams[sessionId]);
 const skillRows=useMemo(()=>skillRowsFrom(skills),[skills]);
 const workspace=workspaces.find(w=>w.id===workspaceId),session=sessions.find(s=>s.id===sessionId);
 const visibleSessions=useMemo(()=>filterByTitle(allSessions,query),[allSessions,query]);
 const projectName=(id:string)=>workspaces.find(x=>x.id===id)?.name??'';
 const slashToken=commandTokenUnderCursor(text,cursor);
 const activeSlash=slashToken&&slashToken!==dismissedToken&&sessionId&&!sessionBusy?(()=>{const items=filterSlashCommands(slashToken);return items.length?{token:slashToken,items}:null})():null;
 const skillToken=useMemo(()=>sessionId&&!sessionBusy?skillTokenUnderCursor(text,cursor):null,[text,cursor,sessionId,sessionBusy]);
 const activeSkill=skillToken&&skillToken.token!==dismissedToken?(()=>{const items=filterSkillRows(skillRows,skillToken.token.slice(1));return items.length?{tok:skillToken,items}:null})():null;
 useEffect(()=>{setSlashIndex(0)},[slashToken,skillToken]);
 // 补全弹层滚动跟随：选中项或候选集变化时把选中项滚入可视区（容器 overflow:auto 不会自动跟随；jsdom 无实现，?. 跳过）。
 useEffect(()=>{selectedOptionRef.current?.scrollIntoView?.({block:'nearest'})},[slashIndex,activeSlash,activeSkill]);
 const taskCount=useMemo(()=>{const open=new Set<string>();for(const e of events){const id=e.data?.id;if(typeof id!=='string')continue;if(e.kind==='job.started')open.add(id);else if(e.kind==='job.finished')open.delete(id)}return open.size},[events]);
 const generation=useRef(0),end=useRef<HTMLDivElement>(null),activeWorkspaceId=useRef(workspaceId);
 const creatingSessions=useRef(new Map<string,Promise<SessionInfo>>());
 activeWorkspaceId.current=workspaceId;
 async function attempt<T>(action:()=>Promise<T>):Promise<T|undefined>{setError('');try{return await action()}catch(e){setError(errorText(e));return undefined}}
 useEffect(()=>{if(!desktop())return;void attempt(async()=>{const [ws,ms,ps,sel,pset,amode]=await Promise.all([command<WorkspaceInfo[]>('workspace_list'),command<ModelConfig[]>('model_list'),command<ProviderInfo[]>('provider_list'),command<DefaultSelection|null>('default_selection_get'),command<PresetInfo[]>('preset_list'),command<string>('approval_mode_get')]);setWorkspaces(ws);setWorkspaceId(ws[0]?.id??'');setModels(ms);setProviders(ps);setPresets(pset);setApprovalMode((KNOWN_APPROVALS as readonly string[]).includes(amode)?amode as ApprovalPolicy:'ask');
  // 恢复上次选择；模型或供应商已停用/删除时回退到第一个可用模型，档位回落最高档。
  const usable=ms.filter(m=>m.enabled&&ps.some(p=>p.id===m.providerId&&p.enabled));
  const saved=sel&&usable.some(m=>m.id===sel.modelId)?sel:null;
  const restored=saved?usable.find(m=>m.id===saved.modelId)??usable[0]:usable[0];
  const restoredEffort=restored
    ?(saved&&saved.modelId===restored.id&&saved.effort&&restored.efforts.includes(saved.effort)?saved.effort:restored.efforts.at(-1)??null)
    :null;
  setModelId(restored?.id??'');setEffort(restoredEffort)})},[]);
 useEffect(()=>{
  const token=++generation.current,targetWorkspaceId=workspaceId;
  let disposed=false;
  const current=()=>!disposed&&token===generation.current&&activeWorkspaceId.current===targetWorkspaceId;
  setSessions([]);setSessionId('');setText('');setSessionBusy(!!targetWorkspaceId);setError('');
  if(!targetWorkspaceId)return;
  void (async()=>{
   try{
    // 侧栏「最近 / 项目」视图需要跨工作区会话；当前工作区列表由全量过滤派生。
    const full=await command<SessionInfo[]>('session_list',{workspaceId:targetWorkspaceId,allWorkspaces:true});
    if(!current())return;
    setAllSessions(full);
    let list=full.filter(s=>s.workspaceId===targetWorkspaceId);
    if(!list.length){
     const created=await createSessionFor(targetWorkspaceId);
     setAllSessions(old=>[created,...old.filter(s=>s.id!==created.id)]);
     if(!current())return;
     list=[created];
    }
    setSessions(list);
    const jump=pendingJump.current;
    if(jump&&list.some(s=>s.id===jump)){pendingJump.current='';setSessionId(jump)}
    else setSessionId(list[0]?.id??'');
   }catch(e){if(current())setError(errorText(e))}
   finally{if(current())setSessionBusy(false)}
  })();
  return()=>{disposed=true};
 },[workspaceId]);
 useEffect(()=>{if(!sessionId)return;const id=sessionId;void attempt(async()=>{const list=await command<SessionEvent[]>('session_events',{sessionId:id});useChat.getState().replace(id,list)})},[sessionId]);
 useEffect(()=>{if(!desktop())return;let alive=true;const refresh=()=>{command<ApprovalRequest[]>('approval_list').then(list=>{if(alive)setApprovals(list)}).catch(e=>{if(alive)setError(errorText(e))});command<string[]>('session_active_list').then(ids=>{if(alive)setActiveIds(new Set(ids))}).catch(()=>{})};void refresh();const timer=setInterval(refresh,2000);return()=>{alive=false;clearInterval(timer)}},[]);
 useEffect(()=>{end.current?.scrollIntoView?.({behavior:'smooth'})},[events,stream]);
 // 打开工作区两步走：先选目录，再在命名弹窗确认（名称预填目录名，留空则后端回退目录名）。
 async function openWorkspace(){setBusy(true);await attempt(async()=>{const path=await open({directory:true,multiple:false,title:'选择原项目目录'});if(path)setWsDialog({mode:'create',path,name:basename(path)})});setBusy(false)}
 async function createWorkspace(){
  if(!wsDialog||wsDialog.mode!=='create')return;
  const {path,name}=wsDialog;
  setWsDialog(null);
  await attempt(async()=>{
   const ws=await command<WorkspaceInfo>('workspace_open',{path,name:name.trim()||null});
   setWorkspaces(old=>[...old.filter(w=>w.id!==ws.id),ws]);
   setWorkspaceId(ws.id);
   setNotice(`已创建托管工作副本「${ws.name}」。原项目仅在确认同步后写入。`);
  });
 }
 async function renameWorkspace(){
  if(!wsDialog||wsDialog.mode!=='rename')return;
  const {id,name}=wsDialog;
  setWsDialog(null);
  await attempt(async()=>{
   const ws=await command<WorkspaceInfo>('workspace_rename',{workspaceId:id,name});
   setWorkspaces(old=>old.map(w=>w.id===ws.id?ws:w));
   setNotice(`项目已重命名为「${ws.name}」。`);
  });
 }
 function createSessionFor(targetWorkspaceId:string):Promise<SessionInfo>{
  const pending=creatingSessions.current.get(targetWorkspaceId);
  if(pending)return pending;
  const request=command<SessionInfo>('session_create',{workspaceId:targetWorkspaceId,title:'新会话'}).then(s=>{
   if(s.workspaceId!==targetWorkspaceId||!s.id)throw new Error('创建的会话不属于当前工作区。');
   return s;
  });
  creatingSessions.current.set(targetWorkspaceId,request);
  const cleanup=()=>{if(creatingSessions.current.get(targetWorkspaceId)===request)creatingSessions.current.delete(targetWorkspaceId)};
  void request.then(cleanup,cleanup);
  return request;
 }
 async function newSession(){
  if(!workspaceId||sessionBusy)return;
  const targetWorkspaceId=workspaceId,token=generation.current;
  const current=()=>token===generation.current&&activeWorkspaceId.current===targetWorkspaceId;
  setSessionBusy(true);setError('');
  try{const s=await createSessionFor(targetWorkspaceId);setAllSessions(old=>[s,...old.filter(item=>item.id!==s.id)]);if(!current())return;setSessions(old=>[s,...old.filter(item=>item.id!==s.id)]);setSessionId(s.id);setText('')}
  catch(e){if(current())setError(errorText(e))}
  finally{if(current())setSessionBusy(false)}
 }
 // 项目组头新建聊天：非当前项目先为该项目创建会话，再经 pendingJump 切换工作区并定位到新会话。
 async function newSessionIn(targetWorkspaceId:string){
  if(!targetWorkspaceId||sessionBusy)return;
  if(targetWorkspaceId===workspaceId){await newSession();return}
  await attempt(async()=>{
   const s=await createSessionFor(targetWorkspaceId);
   pendingJump.current=s.id;
   setWorkspaceId(targetWorkspaceId);
  });
 }
 async function addAttachment(){
  const id=sessionId,workspace=workspaceId,epoch=attachmentEpoch.current;
  if(!id||importing)return;setImporting(true);
  const current=()=>epoch===attachmentEpoch.current&&activeSessionId.current===id&&activeWorkspaceId.current===workspace;
  try{const path=await open({multiple:false,directory:false,title:'选择附件（最多20MiB）'});if(!path||!current())return;const artifact=await command<ArtifactMetadata>('artifact_import',{sessionId:id,path});if(current())setAttachments(old=>old.some(a=>a.hash===artifact.hash)?old:[...old,artifact])}
  catch(e){if(current())setError(errorText(e))}finally{if(current())setImporting(false)}
 }
 // 切换模型：当前档位被新模型支持则保留，否则回落其最高档（默认档位即最高档）；
 // 同时静默持久化默认选择，并在会话流插入临时切换标记（不落库）。
 function chooseModel(id:string){
  setPresetId('');
  const next=models.find(m=>m.id===id);
  const kept=next&&next.efforts.length?(effort&&next.efforts.includes(effort)?effort:next.efforts.at(-1)??null):null;
  const from=models.find(m=>m.id===modelId);
  setModelId(id);setEffort(kept);
  void command('default_selection_save',{selection:{modelId:id,effort:kept}}).catch(()=>{});
  if(from&&from.id!==id&&sessionId)addModelNote(`模型已切换：${from.name||from.model} → ${next?.name||next?.model||id} · 档位 ${kept??'默认'}`);
 }
 function chooseEffort(value:string){setPresetId('');const next=value===''?null:value;setEffort(next);void command('default_selection_save',{selection:{modelId,effort:next}}).catch(()=>{})}
 function addModelNote(text:string){const id=sessionId;if(id)setModelNotes(old=>[...old,{sessionId:id,text}])}
 // 选择预设：以预设字段填充级联（级联持久化沿用上次模型选择）；手改级联即放弃预设。
 function choosePreset(id:string){
  setPresetId(id);
  const info=presets.find(p=>p.id===id);
  if(!id||!info)return;
  if(info.broken){setNotice(`预设“${info.name}”不可用（${info.broken}），未改变模型选择。`);setPresetId('');return}
  if(info.modelId&&!models.some(m=>m.id===info.modelId)){setNotice(`预设“${info.name}”引用的模型不在当前目录，未改变模型选择。`);setPresetId('');return}
  if(info.modelId){
   const nextEffort=info.effort??models.find(m=>m.id===info.modelId)?.effort??null;
   setModelId(info.modelId);setEffort(nextEffort);
   void command('default_selection_save',{selection:{modelId:info.modelId,effort:nextEffort}}).catch(()=>{});
  }
 }
 // 权限模式：保存为默认档并对当前会话即时生效；会话内显式切换记录覆盖（与其他会话隔离）。
 const approvalModeForSession=(id:string)=>approvalOverrides[id]??approvalMode;
 function chooseApprovalMode(mode:ApprovalPolicy){
  setApprovalMode(mode);
  if(sessionId)setApprovalOverrides(old=>({...old,[sessionId]:mode}));
  void attempt(async()=>{await command('approval_mode_save',{mode,sessionId:sessionId||null})});
 }
 async function dispatchText(input:string,hashes:string[]){const id=sessionId;setRunning(old=>({...old,[id]:true}));await attempt(async()=>{if(hashes.length)await send(id,input,modelId,useChat.getState().receive,hashes,effort);else await send(id,input,modelId,useChat.getState().receive,undefined,effort)});useChat.getState().clearStream(id);try{useChat.getState().replace(id,await command<SessionEvent[]>('session_events',{sessionId:id}))}catch(e){setError(errorText(e))}setRunning(old=>({...old,[id]:false}))}
 async function submit(e:FormEvent){e.preventDefault();if(sessionBusy||importing||!sessionId||(!text.trim()&&!attachments.length)||running[sessionId]||!modelId)return;const expanded=expandSlashPrompt(text);if(expanded?.kind==='usage'){setNotice(expanded.name==='plan'?PLAN_USAGE:GOAL_USAGE);return}const hashes=attachments.map(a=>a.hash);setText('');setAttachments([]);await dispatchText(expanded?expanded.text:text,hashes)}
 // 加号菜单中的计划/目标项：只插入斜杠命令，参数由用户补齐后经 submit 展开为预置提示词。
 function insertPlusCommand(name:'plan'|'goal'){setPlusOpen(false);const next=`/${name} `;setText(next);setCursor(next.length);setDismissedToken('');textareaRef.current?.focus();requestAnimationFrame(()=>{textareaRef.current?.setSelectionRange(next.length,next.length)})}
 function applyCompletion(spec:SlashSpec,token:string){const rest=text.slice(token.length);const next=`/${spec.name}${rest||' '}`;setText(next);setCursor(next.length);setDismissedToken('');requestAnimationFrame(()=>{textareaRef.current?.setSelectionRange(next.length,next.length)})}
 // 技能补全：替换光标处 $token 为 $名称（token 后已有空格不重复补），同一 token 内不再弹窗。
 function applySkillCompletion(row:SkillRow,tok:SkillToken){const completed=`$${row.name}`;const needsSpace=text[tok.end]!==' ';const next=text.slice(0,tok.start)+completed+(needsSpace?' ':'')+text.slice(tok.end);const caret=tok.start+completed.length+(needsSpace?1:0);setText(next);setCursor(caret);setDismissedToken(`$${row.name}`);requestAnimationFrame(()=>{textareaRef.current?.setSelectionRange(caret,caret)})}
 async function executeSlash(spec:SlashSpec){
  setText('');setCursor(0);setDismissedToken('');
  if(spec.name==='help'){setNotice(`可用命令：${SLASH_COMMANDS.map(item=>`/${item.name}（${item.description}）`).join('、')}。输入 / 唤出补全。`);return}
  if(spec.name==='status'){setNotice(`会话 ${sessionId} · 模型 ${models.find(item=>item.id===modelId)?.name??modelId} · 工作副本已隔离，改动需审阅后同步。`);return}
  if(spec.name==='model'){setModelMenuOpen(true);return}
  if(spec.name==='diff'){useSidePanel.getState().openChanges();return}
  if(spec.name==='plan'||spec.name==='goal'){setNotice(spec.name==='plan'?PLAN_USAGE:GOAL_USAGE);return}
  if(spec.name==='clear'){await newSession();return}
  if(!sessionId||sessionBusy||running[sessionId]||!modelId)return;
  if(spec.name==='init'){await dispatchText(INIT_PROMPT,[]);setNotice('已发起 AGENTS.md 生成；完成后用“查看更改”审阅并同步到原项目。');return}
  if(spec.name==='compact'){setNotice('正在压缩会话历史……');await attempt(async()=>{await command('session_compact',{sessionId,modelId,effort});useChat.getState().replace(sessionId,await command<SessionEvent[]>('session_events',{sessionId}));setNotice('历史已压缩为摘要，后续回合将基于摘要继续。')});return}
 }
 async function cancel(){await attempt(()=>command('session_cancel',{sessionId}))}
 async function openSession(id:string){if(!workspaceId||id===sessionId)return;await attempt(async()=>{let target=sessions.find(s=>s.id===id);if(!target){target=(await command<SessionInfo[]>('session_list',{workspaceId})).find(s=>s.id===id)}if(!target)return;if(target.workspaceId!==workspaceId)throw new Error('子会话不属于当前工作区。');setSessions(old=>old.some(s=>s.id===target!.id)?old:[target!,...old]);setSessionId(target.id);setText('')})}
 function openFromPanel(id:string,workspace:string){setPanel(null);if(workspace===workspaceId){void openSession(id);return}pendingJump.current=id;setWorkspaceId(workspace)}
 // 侧栏跨工作区点击：同工作区直接打开，跨工作区先切工作区再经 pendingJump 跳转。
 function openFromSidebar(s:SessionInfo){if(s.workspaceId===workspaceId){void openSession(s.id);setText('');return}openFromPanel(s.id,s.workspaceId)}
 // 置顶切换：乐观更新，失败回滚并提示。
 async function togglePinned(target:WorkspaceInfo){
  const next=!target.pinned;
  setWorkspaces(old=>old.map(w=>w.id===target.id?{...w,pinned:next}:w));
  try{await command<WorkspaceInfo>('workspace_set_pinned',{workspaceId:target.id,pinned:next})}
  catch(e){setWorkspaces(old=>old.map(w=>w.id===target.id?{...w,pinned:!next}:w));setError(errorText(e))}
 }
 function chooseView(mode:ViewMode){setViewMode(mode);try{localStorage.setItem('sessionView',mode)}catch{}}
 function toggleCollapsed(id:string){setCollapsed(old=>{const next=old.includes(id)?old.filter(x=>x!==id):[...old,id];try{localStorage.setItem('sessionCollapsed',JSON.stringify(next))}catch{}return next})}
 async function resolve(id:string,allow:boolean){await attempt(async()=>{await command('approval_resolve',{id,allow});setApprovals(old=>old.filter(a=>a.id!==id))})}
 async function exportSession(){await attempt(async()=>{const path=await save({defaultPath:'rust-harness-session.json',filters:[{name:'JSON',extensions:['json']}]});if(!path)return;await command('session_export',{sessionId,path});setNotice('会话已导出。')})}
 return <div className="app">
  <aside className="sidebar">
   <div className="brand"><span className="brand-mark"><Terminal size={23}/></span><span>Rust Harness<small>你的本地开发搭档</small></span><span className="version">α</span></div>
   <button className="new-chat" title="新建聊天" onClick={newSession} disabled={!workspaceId||sessionBusy}><Plus size={16}/><span>新建聊天</span></button>
   <div className="session-search"><Search size={14} aria-hidden="true"/><input value={query} onChange={e=>setQuery(e.target.value)} placeholder="搜索会话…" aria-label="搜索会话"/></div>
   <div className="section-label"><span>任务列表</span><span className="label-spacer"/><button type="button" role="switch" aria-checked={viewMode==='project'} aria-label="切换视图：最近 / 项目" className="view-switch" onClick={()=>chooseView(viewMode==='recent'?'project':'recent')}><span className="knob" aria-hidden="true"/><span className="sw-label a" aria-hidden="true">最近</span><span className="sw-label b" aria-hidden="true">项目</span></button></div>
   <nav className="sessions">
    {viewMode==='recent'?groupByTime(visibleSessions,new Date()).map(group=><Fragment key={group.group}>
     <div className="group-head">{GROUP_LABELS[group.group]}</div>
     {group.sessions.map(s=>{const ws=workspaces.find(w=>w.id===s.workspaceId);return <button key={s.id} className={`session-row${sessionId===s.id?' active':''}${s.parentId?' child':''}`} title={s.parentId?'子代理会话':undefined} onClick={()=>openFromSidebar(s)}><MessageSquare size={16}/><span className="session-lines"><span className="session-l1"><span className="session-title">{s.title}</span>{s.parentId&&<em className="subagent-badge">子任务</em>}<span className="session-time">{relativeLabel(s.updatedAt,new Date())}</span></span><span className="session-l2">{ws&&<span>{projectName(s.workspaceId)}</span>}{ws?.pinned&&<Pin size={10} aria-hidden="true"/>}{(running[s.id]||activeIds.has(s.id))&&<i className="live-dot" aria-label="运行中"/>}</span></span></button>})}
    </Fragment>):groupByProject(visibleSessions,workspaces).map(({workspace:ws,sessions:list})=>{
     const open=!collapsed.includes(ws.id);
     return <div key={ws.id} className="ws-group">
      <div className="ws-head">
       <button type="button" className="ws-toggle" aria-expanded={open} onClick={()=>toggleCollapsed(ws.id)}><ChevronRight size={13} className="chev" aria-hidden="true"/><span className="ws-name">{ws.name}</span><span className="ws-count">{list.length}</span></button>
       <button type="button" className="ws-add" aria-label={`在 ${ws.name} 新建聊天`} title="新建聊天" disabled={sessionBusy} onClick={()=>void newSessionIn(ws.id)}><Plus size={13}/></button>
       <button type="button" className="ws-rename" aria-label={`重命名 ${ws.name}`} title="重命名项目" onClick={()=>setWsDialog({mode:'rename',id:ws.id,name:ws.name})}><Pencil size={13}/></button>
       <button type="button" className={`ws-pin${ws.pinned?' on':''}`} aria-label={`${ws.pinned?'取消置顶':'置顶'} ${ws.name}`} title={ws.pinned?'取消置顶':'置顶项目'} onClick={()=>void togglePinned(ws)}><Pin size={13}/></button>
      </div>
      {open&&list.map(s=><button key={s.id} className={`session-row one${sessionId===s.id?' active':''}${s.parentId?' child':''}`} title={s.parentId?'子代理会话':undefined} onClick={()=>openFromSidebar(s)}><MessageSquare size={16}/><span className="session-title">{s.title}</span>{s.parentId&&<em className="subagent-badge">子任务</em>}{(running[s.id]||activeIds.has(s.id))&&<i className="live-dot" aria-label="运行中"/>}<span className="session-time">{relativeLabel(s.updatedAt,new Date())}</span></button>)}
     </div>;
    })}
    {!visibleSessions.length&&<p className="muted empty-small">{query?'无匹配会话。':workspaceId?'还没有会话，点击「新建聊天」开始':'打开工作区后，会话将保存在本机。'}</p>}
    {!visibleSessions.length&&!query&&!workspaceId&&<button className="empty-open" onClick={openWorkspace} disabled={busy||!desktop()}><FolderOpen size={14}/>打开工作区</button>}
   </nav>
   <div className="sidebar-bottom"><div className="local-badge"><i className="live-dot"/> 本地优先 · 无遥测</div><button onClick={()=>setPanel('plans')}><Target size={17}/>计划与目标</button><button onClick={()=>setPanel('presets')}><Bookmark size={17}/>能力预设</button><button onClick={()=>setPanel('skills')}><Sparkles size={17}/>技能</button><button onClick={()=>setPanel('automations')}><Timer size={17}/>自动化{automationCount!==null&&<span className="count">{automationCount}</span>}</button><button onClick={()=>setPanel('plugins')}><Unplug size={17}/>插件与权限</button><button onClick={()=>setPanel('models')}><Settings2 size={17}/>模型设置 <span className="count">{models.length}</span></button></div>
  </aside>
  <main className="main"><header className="topbar"><div className="breadcrumb"><Folder size={15}/>{workspace?workspace.name:'工作台'}<span>/</span><strong>{session?.title??'新的可能，从这里开始'}</strong></div><div className="toolbar"><button className="icon-button" title="导出会话" aria-label="导出会话" disabled={!sessionId} onClick={exportSession}><Download size={17}/></button><button className="soft" aria-expanded={panel==='tasks'} aria-controls="task-popover" onClick={()=>setPanel(p=>p==='tasks'?null:'tasks')} disabled={!sessionId}><ListTodo size={16}/>后台任务{taskCount>0&&<span className="count">{taskCount}</span>}</button><button className="soft" aria-expanded={sideOpen} onClick={()=>useSidePanel.getState().toggle()} disabled={!workspaceId}><GitCompareArrows size={16}/>查看更改</button>{panel==='tasks'&&sessionId&&<BackgroundTasksPanel sessionId={sessionId} onClose={()=>setPanel(null)}/>}</div></header>
   {!desktop()&&<div className="host-banner"><Terminal size={16}/>当前为界面预览，需要桌面宿主才能访问工作区和模型。</div>}
   {error&&<div role="alert" className="alert"><span>{error}</span><button aria-label="关闭错误" onClick={()=>setError('')}><X size={16}/></button></div>}
   {notice&&<div role="status" className="notice"><Check size={15}/>{notice}<button aria-label="关闭提示" onClick={()=>setNotice('')}><X size={15}/></button></div>}
   <section className="conversation" aria-label="聊天消息">
    {!events.length&&!stream&&<div className="welcome"><span className="welcome-icon"><Terminal size={35}/></span><div className="eyebrow">BUILD WITH CONFIDENCE</div><h1>想把什么变成现实？</h1><p>理解代码、探索想法，或完成下一次改进。<br/>在独立的工作副本中协作，每一处更改都由你掌控。</p>{!workspaceId&&<button className="primary welcome-open" onClick={openWorkspace} disabled={busy||!desktop()}><FolderOpen size={15}/>打开工作区</button>}<div className="suggestions">{['梳理项目结构和关键模块','检查代码，找出值得改进的地方','帮我规划并实现一个新功能'].map((s,i)=><button key={s} onClick={()=>setText(s)} disabled={!sessionId}><span>0{i+1}</span>{s}<ArrowUp size={14}/></button>)}</div><div className="welcome-foot"><ShieldCheck size={15}/>工作副本隔离 · 操作可审批 · 更改可预览</div></div>}
    <div className="messages"><ConversationEvents events={events} workspaceId={workspaceId} onOpenSession={openSession} activeIds={activeIds}/>{modelNotes.filter(n=>n.sessionId===sessionId).map((note,index)=><div key={`${note.sessionId}-${index}`} className="model-note" role="status">{note.text}</div>)}{stream&&<article className="message assistant"><div className="avatar">R</div><div className="message-body"><div className="message-label">Rust Harness <span className="stream-label">正在思考与执行</span></div>{stream.reasoning&&<details className="reasoning"><summary>思考过程</summary><pre>{stream.reasoning}</pre></details>}<div className="prose"><Markdown content={stream.text||'正在处理…'}/><span className="cursor"/></div></div></article>}<div ref={end}/></div>
   </section>
   <footer className="composer-area"><form className="composer" onSubmit={submit}>{activeSkill&&<div className="slash-menu" role="listbox" aria-label="技能补全">{activeSkill.items.map((row,index)=>{const selected=index===Math.min(slashIndex,activeSkill.items.length-1);return <button type="button" key={row.name} role="option" aria-selected={selected} ref={selected?selectedOptionRef:undefined} className={selected?'selected':undefined} onMouseDown={e=>{e.preventDefault();applySkillCompletion(row,activeSkill.tok)}}><strong>${row.name}</strong><span>{row.description||'（无描述）'}</span></button>})}</div>}{activeSlash&&<div className="slash-menu" role="listbox" aria-label="斜杠命令补全">{activeSlash.items.map((spec,index)=>{const selected=index===Math.min(slashIndex,activeSlash.items.length-1);return <button type="button" key={spec.name} role="option" aria-selected={selected} ref={selected?selectedOptionRef:undefined} className={selected?'selected':undefined} onMouseDown={e=>{e.preventDefault();void executeSlash(spec)}}><strong>/{spec.name}</strong><span>{spec.description}</span></button>})}</div>}<textarea aria-label="消息" ref={textareaRef} placeholder={sessionBusy?'正在准备会话…':sessionId?'描述任务，或继续你的想法…（输入 / 唤出命令）':workspaceId?'点击“新建聊天”重试，开始协作':'打开工作区，开始协作'} value={text} onChange={e=>{setText(e.target.value);setCursor(e.currentTarget.selectionStart??0)}} onSelect={e=>setCursor(e.currentTarget.selectionStart??0)} disabled={!sessionId||sessionBusy} onKeyDown={e=>{const menu=activeSlash,menuSkill=activeSkill;if(menu){if(e.key==='ArrowDown'){e.preventDefault();setSlashIndex(menu.items.length?(slashIndex+1)%menu.items.length:0);return}if(e.key==='ArrowUp'){e.preventDefault();setSlashIndex(menu.items.length?(slashIndex-1+menu.items.length)%menu.items.length:0);return}if(e.key==='Tab'){e.preventDefault();applyCompletion(menu.items[Math.min(slashIndex,menu.items.length-1)],menu.token);return}if(e.key==='Escape'){e.preventDefault();setDismissedToken(menu.token);return}if(e.key==='Enter'&&!e.shiftKey&&!e.nativeEvent.isComposing){e.preventDefault();const spec=menu.items[Math.min(slashIndex,menu.items.length-1)];if(spec)void executeSlash(spec);return}}if(menuSkill){if(e.key==='ArrowDown'){e.preventDefault();setSlashIndex(menuSkill.items.length?(slashIndex+1)%menuSkill.items.length:0);return}if(e.key==='ArrowUp'){e.preventDefault();setSlashIndex(menuSkill.items.length?(slashIndex-1+menuSkill.items.length)%menuSkill.items.length:0);return}if(e.key==='Tab'||e.key==='Enter'){e.preventDefault();const row=menuSkill.items[Math.min(slashIndex,menuSkill.items.length-1)];if(row)applySkillCompletion(row,menuSkill.tok);return}if(e.key==='Escape'){e.preventDefault();setDismissedToken(menuSkill.tok.token);return}}if(e.key==='Enter'&&!e.shiftKey&&!e.nativeEvent.isComposing){e.preventDefault();e.currentTarget.form?.requestSubmit()}}}/>{attachments.length>0&&<div className="permission-tags">{attachments.map(a=><span key={a.hash}>{a.name} <button type="button" aria-label={`移除附件 ${a.name}`} onClick={()=>setAttachments(old=>old.filter(item=>item.hash!==a.hash))}><X size={12}/></button></span>)}</div>}<div className="composer-bottom"><div className="composer-plus-wrap"><button type="button" className="composer-plus" aria-label="添加内容" aria-haspopup="menu" aria-expanded={plusOpen} disabled={!sessionId||sessionBusy||!!running[sessionId]} onClick={()=>setPlusOpen(v=>!v)}><Plus size={17}/></button>{plusOpen&&<div className="plus-menu" role="menu" aria-label="添加内容"><button type="button" role="menuitem" disabled={importing||attachments.length>=20} onClick={()=>{setPlusOpen(false);void addAttachment()}}><Paperclip size={15}/><span><strong>附件</strong><small>选择本地文件，随消息发送（最多 20 MiB）</small></span></button><button type="button" role="menuitem" onClick={()=>insertPlusCommand('plan')}><ListTodo size={15}/><span><strong>计划</strong><small>插入 /plan，让模型先制定实施计划</small></span></button><button type="button" role="menuitem" onClick={()=>insertPlusCommand('goal')}><Target size={15}/><span><strong>目标</strong><small>插入 /goal，为本会话设定一个目标</small></span></button></div>}</div><div className="model-select"><div className="mode-select"><select aria-label="权限模式" value={sessionId?approvalModeForSession(sessionId):approvalMode} onChange={e=>chooseApprovalMode(e.target.value as ApprovalPolicy)}>{APPROVAL_MODES.map(m=><option key={m.value} value={m.value} title={m.description}>{m.label}</option>)}</select></div><select aria-label="能力预设" value={presetId} onChange={e=>choosePreset(e.target.value)}><option value="">不使用预设</option>{presets.filter(p=>p.id!=='builtin:default').map(p=><option key={p.id} value={p.id} disabled={!!p.broken}>{p.name}{p.broken?'（不可用）':''}</option>)}</select><ModelSelect models={models} providers={providers} modelId={modelId} effort={effort} open={modelMenuOpen} onOpenChange={setModelMenuOpen} onPickModel={chooseModel} onPickEffort={chooseEffort} onOpenManage={()=>setPanel('models')}/></div><UsageRing models={models} providers={providers} modelId={modelId} sessionId={sessionId}/><div className="send-controls"><span>Enter 发送 · Shift + Enter 换行</span>{running[sessionId]?<button type="button" className="send" aria-label="取消生成" onClick={cancel}><Square size={16}/></button>:<button className="send" type="submit" aria-label="发送消息" disabled={sessionBusy||importing||(!text.trim()&&!attachments.length)||!sessionId||!modelId}><ArrowUp size={19}/></button>}</div></div></form><div className="footer-note">原项目保持安全。模型生成的更改需要你确认后才会同步。</div></footer>
  </main>
  <SidePanel workspaceId={workspaceId}/>
  {panel==='models'&&<ModelPanel models={models} onClose={()=>setPanel(null)} onSaved={async()=>{const [ms,ps]=await Promise.all([command<ModelConfig[]>('model_list'),command<ProviderInfo[]>('provider_list')]);setModels(ms);setProviders(ps);if(!ms.some(m=>m.id===modelId)||!ms.find(m=>m.id===modelId)?.enabled){const usable=ms.filter(m=>m.enabled&&ps.some(p=>p.id===m.providerId&&p.enabled));setModelId(usable[0]?.id??'');setEffort(usable[0]?.efforts.at(-1)??null)}}}/>}
  {panel==='presets'&&<PresetsPanel models={models} providers={providers} onClose={()=>setPanel(null)} onChanged={async()=>{setPresets(await command<PresetInfo[]>('preset_list'))}}/>}
  {panel==='skills'&&<SkillsPanel workspaceId={workspaceId} onClose={()=>setPanel(null)}/>}
  <PluginThemeHost workspaceId={workspaceId} revision={pluginRevision}/>
  {panel==='plugins'&&<PluginPanel workspaceId={workspaceId} sessionId={sessionId} revision={pluginRevision} onChanged={()=>setPluginRevision(v=>v+1)} onClose={()=>setPanel(null)}/>}
  {panel==='plans'&&<PlansPanel currentSessionId={sessionId} onOpenSession={openFromPanel} onClose={()=>setPanel(null)}/>}
  {panel==='automations'&&<Modal title="自动化任务" onClose={()=>setPanel(null)}><AutomationsPanel workspaceId={workspaceId} sessions={sessions} onChanged={()=>{if(workspaceId)command<unknown[]>('automation_list',{workspaceId}).then(list=>setAutomationCount(list.length)).catch(()=>{})}}/></Modal>}
  {wsDialog&&<Modal title={wsDialog.mode==='create'?'打开工作区':'重命名项目'} onClose={()=>setWsDialog(null)}><form className="settings-form" onSubmit={e=>{e.preventDefault();void (wsDialog.mode==='create'?createWorkspace():renameWorkspace())}}>
   {wsDialog.mode==='create'&&<p className="muted">{wsDialog.path}</p>}
   <label>项目名称<input value={wsDialog.name} onChange={e=>setWsDialog(old=>old&&{...old,name:e.target.value})} placeholder="默认使用目录名"/></label>
   {wsDialog.mode==='create'&&<p className="muted">留空则使用目录名「{basename(wsDialog.path)}」。</p>}
   <div className="actions"><button type="button" onClick={()=>setWsDialog(null)}>取消</button><button className="primary" type="submit">{wsDialog.mode==='create'?'创建':'保存'}</button></div>
  </form></Modal>}
  {approvals[0]&&<Modal title="操作需要你的批准" onClose={()=>void resolve(approvals[0].id,false)}><div className="approval-heading"><ShieldCheck size={24}/><strong>{approvals[0].tool}</strong></div><p>{approvals[0].reason}</p><pre className="code-block">{printable(approvals[0].arguments)}</pre><p className="muted">会话：{sessions.find(s=>s.id===approvals[0].sessionId)?.title??approvals[0].sessionId}</p><div className="actions"><button onClick={()=>resolve(approvals[0].id,false)}>拒绝</button><button className="primary" onClick={()=>resolve(approvals[0].id,true)}>批准此次操作</button></div></Modal>}
 </div>
}
const EMPTY_EVENTS:SessionEvent[]=[];
function Markdown({content}:{content:string}){return <ReactMarkdown remarkPlugins={[remarkGfm]}>{content}</ReactMarkdown>}
function durationLabel(start?:SessionEvent,end?:SessionEvent){
 const startAt=Date.parse(start?.createdAt??''),endAt=Date.parse(end?.createdAt??'');
 if(!Number.isFinite(startAt)||!Number.isFinite(endAt)||endAt<startAt)return '本轮已完成';
 const seconds=Math.floor((endAt-startAt)/1000),hours=Math.floor(seconds/3600),minutes=Math.floor(seconds%3600/60),remaining=seconds%60;
 return `本轮已完成，用时${hours?`${hours}小时`:''}${hours||minutes?`${minutes}分`:''}${remaining}秒`;
}
type EventCardExtras={workspaceId?:string;onOpenSession?:(id:string)=>void;activeIds?:Set<string>};
function CompletedTurn({events,...extras}:{events:SessionEvent[]}&EventCardExtras){
 const finalIndex=events.map(e=>e.kind).lastIndexOf('assistant.message');
 const process=events.filter((event,index)=>index!==finalIndex&&!['turn.start','turn.end','user.message'].includes(event.kind));
 const user=events.find(event=>event.kind==='user.message'),final=finalIndex>=0?events[finalIndex]:undefined;
 const start=events.find(event=>event.kind==='turn.start'),end=events.at(-1);
 return <>{user&&<EventCard event={user} {...extras}/>} {process.length>0&&<details className="turn-details" open><summary>{durationLabel(start,end)}<ChevronDown size={14}/></summary>{process.map(event=><EventCard key={event.id} event={event} {...extras}/>)}</details>} {final&&<EventCard event={final} {...extras}/>}</>;
}
function ConversationEvents({events,...extras}:{events:SessionEvent[]}&EventCardExtras){
 const cards:ReactNode[]=[];let start=-1;
 for(let index=0;index<events.length;index++){
  const event=events[index];
  if(event.kind==='turn.start'){if(start>=0)cards.push(<Fragment key={`open-${events[start].id}`}>{events.slice(start,index).map(item=><EventCard key={item.id} event={item} {...extras}/>)}</Fragment>);start=index;continue}
  if(event.kind==='turn.end'&&start>=0){const turn=events.slice(start,index+1);cards.push(event.data.status==='completed'?<CompletedTurn key={event.id} events={turn} {...extras}/>:<Fragment key={event.id}>{turn.map(item=><EventCard key={item.id} event={item} {...extras}/>)}</Fragment>);start=-1;continue}
  if(start<0)cards.push(<EventCard key={event.id} event={event} {...extras}/>);
 }
 if(start>=0)cards.push(<Fragment key={`open-${events[start].id}`}>{events.slice(start).map(event=><EventCard key={event.id} event={event} {...extras}/>)}</Fragment>);
 return <>{cards}</>;
}
export function EventCard({event,...extras}:{event:SessionEvent}&EventCardExtras) {
 const {kind,data}=event;
 if(kind==='subagent.created'||kind==='subagent.message_sent'){
  const childId=typeof data.sessionId==='string'?data.sessionId:'';
  const sent=kind==='subagent.message_sent';
  return <article className="tool-card subagent-card"><div className="subagent-row">{sent?<Send size={15}/>:<Bot size={15}/>}<strong>{sent?'已向子代理投递任务':'已创建子代理'}</strong>{childId&&data.historyOnly!==true&&extras.onOpenSession&&<button type="button" onClick={()=>extras.onOpenSession?.(childId)}>查看会话</button>}{childId&&extras.activeIds?.has(childId)&&<i className="live-dot" aria-label="子代理运行中"/>}</div>{data.historyOnly===true&&<p className="muted">历史派生记录，原父子关系属于源会话。</p>}{sent&&<pre>{printable(data.content)}</pre>}</article>;
 }
 if(kind==='todo.state'){
  const items=Array.isArray(data.items)?(data.items as Record<string,unknown>[]):[];
  return <article className="tool-card todo-card"><div className="subagent-row"><ListTodo size={15}/><strong>待办清单</strong></div><TodoList items={items}/></article>;
 }
 if(kind==='plan.state'||kind==='goal.state')return <article className="tool-card"><div className="subagent-row">{kind==='plan.state'?<ListTodo size={15}/>:<Target size={15}/>}<strong>{kind==='plan.state'?'计划':'目标'}</strong></div><div className="prose"><Markdown content={printable(data.content)}/></div></article>;
 if(kind==='user.message')return <article className="message user"><div className="avatar">你</div><div className="message-body"><div className="message-label">你</div><div className="prose">{printable(data.content)}</div>{Array.isArray(data.attachments)&&data.attachments.map((a,index)=><ArtifactDownload key={index} sessionId={event.sessionId} artifact={a as ArtifactMetadata}/>)}</div></article>;
 if(kind==='assistant.message'||kind==='assistant.attempt')return <article className="message assistant"><div className="avatar">R</div><div className="message-body"><div className="message-label">Rust Harness</div>{!!data.reasoning&&<details className="reasoning"><summary>思考过程</summary><pre>{printable(data.reasoning)}</pre></details>}<div className="prose"><Markdown content={printable(data.content)}/></div>{!!data.error&&<div className="inline-error">{printable(data.error)}</div>}</div></article>;
 if(kind==='tool.call'||kind==='tool.result'){
  const args=typeof data.arguments==='object'&&data.arguments!==null?data.arguments as Record<string,unknown>:null;
  const toolPath=(String(data.name)==='write'||String(data.name)==='edit')&&typeof args?.path==='string'?args.path:null;
  return <details className="tool-card"><summary><Terminal size={15}/><strong>{printable(data.name)}</strong>{toolPath&&<button type="button" className="file-chip" title={toolPath} onClick={e=>{e.preventDefault();useSidePanel.getState().openPreview(toolPath)}}><FileCode size={12}/>{basename(toolPath)}</button>}<span>{kind==='tool.call'?'工具调用':data.isError?'执行失败':'已完成'}</span></summary>{kind==='tool.result'&&String(data.name).startsWith('plugin__')?<PluginResultCards workspaceId={extras.workspaceId} content={printable(data.content)}/>:<pre>{printable(data.arguments??data.content)}</pre>}{!!data.artifact&&<ArtifactDownload sessionId={event.sessionId} artifact={data.artifact as ArtifactMetadata}/>}</details>;
 }
 if(kind==='plugin.command.result')return <article className="tool-card"><strong>{printable(data.name)}</strong><PluginResultCards workspaceId={extras.workspaceId} content={printable(data.content)}/></article>;
 if(kind==='job.started')return <details className="tool-card"><summary><Terminal size={15}/><strong>{printable(data.command)}</strong><span>{JOB_STATUS[String(data.status)]??'运行中'}</span></summary><pre>{printable(data)}</pre></details>;
 if(kind==='job.finished'){
  const status=String(data.status),output=data.output as {stdout?:unknown;stderr?:unknown;exit_code?:unknown}|null;
  return <details className="tool-card"><summary><Terminal size={15}/><strong>后台任务已结束</strong><span className={status==='completed'?'':'conflict'}>{JOB_STATUS[status]??status}</span></summary>{output?<pre>{`退出码 ${printable(output.exit_code)}\n【stdout】\n${printable(output.stdout??'')}\n【stderr】\n${printable(output.stderr??'')}`}</pre>:data.error!=null&&<pre>错误：{printable(data.error)}</pre>}</details>;
 }
 if(kind==='job.cancel_requested')return <div className="turn-status">已请求取消后台任务 {printable(data.id)}</div>;
 if(kind==='terminal.opened')return <details className="tool-card"><summary><Terminal size={15}/><strong>{`已打开交互式终端 ${printable(data.id)}`}</strong><span>{TERMINAL_STATUS[String(data.status)]??'运行中'}</span></summary><pre>{`shell：${printable(data.shell)}`}</pre></details>;
 if(kind==='terminal.closed'){
  const status=String(data.status);
  return <details className="tool-card"><summary><Terminal size={15}/><strong>{`交互式终端已结束 ${printable(data.id)}`}</strong><span className={status==='exited'?'':'conflict'}>{TERMINAL_STATUS[status]??status}</span></summary><pre>{[data.exitCode!=null?`退出码 ${printable(data.exitCode)}`:'',typeof data.outputBlobHash==='string'?`尾部输出 artifact：${data.outputBlobHash}${data.outputTruncated===true?'（有截断）':''}`:'',data.error!=null?`错误：${printable(data.error)}`:''].filter(Boolean).join('\n')}</pre></details>;
 }
 if(kind==='schedule.created')return <details className="tool-card"><summary><CalendarClock size={15}/><strong>{printable(data.task)}</strong><span>{printable(data.at)}</span></summary><pre>{`执行时间：${printable(data.at)}\n模型：${printable(data.modelId)}`}</pre></details>;
 if(kind==='schedule.deleted')return <div className="turn-status">已删除定时任务 {printable(data.id)}</div>;
 if(kind.startsWith('schedule.')){const status=String(data.status??kind.slice('schedule.'.length));return <div className="turn-status">定时任务 {printable(data.id)}：{SCHEDULE_STATUS[status]??status}</div>}
 if(kind==='skill.loaded')return <details className="tool-card"><summary><Sparkles size={15}/><strong>已加载技能 {printable(data.name)}</strong><span>$提及注入</span></summary><pre>{printable(data.content)}</pre></details>;
 if(kind==='compaction.summary')return <details className="tool-card"><summary>上下文摘要</summary><pre>{printable(data.content)}</pre></details>;
 if(kind==='turn.end'&&data.status&&data.status!=='completed')return <div className="turn-status">本轮状态：{printable(data.status)}</div>;
 return null;
}
function ArtifactDownload({sessionId,artifact}:{sessionId:string;artifact:ArtifactMetadata}) {
 const [error,setError]=useState(''),[busy,setBusy]=useState(false);
 async function download(){setBusy(true);setError('');try{const path=await save({defaultPath:artifact.name});if(path)await command('artifact_save',{sessionId,hash:artifact.hash,path})}catch(e){setError(errorText(e))}finally{setBusy(false)}}
 return <div><button type="button" disabled={busy} aria-label={`下载 ${artifact.name}`} onClick={download}><Download size={14}/>{artifact.name} ({artifact.size} 字节)</button>{error&&<p role="alert">{error}</p>}</div>
}
export function TodoList({items}:{items:Record<string,unknown>[]}) {
 return items.length?<ul className="todo-list">{items.map((item,index)=>{const text=String(item.text??item.content??item.title??printable(item));const done=item.done===true||item.completed===true||item.status==='completed';return <li key={index} className={done?'done':undefined}>{done?'✓':'○'} {text}</li>})}</ul>:<p className="muted">清单为空。</p>;
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
function PluginPanel({workspaceId,sessionId,revision,onChanged,onClose}:{workspaceId:string;sessionId:string;revision:number;onChanged:()=>void;onClose:()=>void}) {
 const [plugins,setPlugins]=useState<PluginInfo[]>([]),[path,setPath]=useState(''),[permissions,setPermissions]=useState(''),[error,setError]=useState(''),[busy,setBusy]=useState(false),[removeId,setRemoveId]=useState('');
 const load=async()=>{setPlugins(await command<PluginInfo[]>('plugin_list'));onChanged()};
 useEffect(()=>{void load().catch(e=>setError(errorText(e)))},[]);
 async function action(fn:()=>Promise<unknown>){setBusy(true);setError('');try{await fn();await load()}catch(e){setError(errorText(e))}finally{setBusy(false)}}
 return <Modal title="插件与权限" onClose={onClose}><p className="muted">从本地导入 WASM 插件。插件仅获得声明、授权与工作区策略的交集权限。</p>{error&&<p role="alert" className="inline-error">{error}</p>}<form className="settings-form" onSubmit={e=>{e.preventDefault();void action(()=>command('plugin_install',{path,permissions:permissions.split(',').map(s=>s.trim()).filter(Boolean)}))}}><label>插件目录或 ZIP 路径<div className="input-row"><input required value={path} onChange={e=>setPath(e.target.value)} placeholder="输入插件完整路径"/><button type="button" disabled={!desktop()} onClick={async()=>{try{const result=await open({multiple:false,directory:true});if(result)setPath(result)}catch(e){setError(errorText(e))}}}>浏览目录</button></div></label><label>授予权限（逗号分隔，默认无权限）<input value={permissions} onChange={e=>setPermissions(e.target.value)} placeholder="按插件清单填写能力名称"/></label><button className="primary" disabled={busy||!desktop()}>导入并授权</button></form><div className="plugin-list">{!plugins.length&&<p className="empty-small muted">暂无已安装插件。</p>}{plugins.map(p=><article key={p.id}><div className="plugin-heading"><Unplug size={19}/><strong>{p.name}</strong><small>{p.version} · {p.kind}</small></div><p>{p.description}</p><div className="permission-tags">{p.permissions.length?p.permissions.map(x=><code key={x}>{x}</code>):<span>无额外授权</span>}</div><div className="plugin-actions"><small>{p.status}</small><button disabled={busy} onClick={()=>action(()=>command('plugin_enable',{id:p.id,enabled:!p.enabled}))}>{p.enabled?'停用':'启用'}</button><button className="danger" disabled={busy} onClick={()=>setRemoveId(p.id)}>移除</button></div>{removeId===p.id&&<div className="confirm">确定移除此插件？<button onClick={()=>setRemoveId('')}>取消</button><button className="danger" onClick={()=>action(async()=>{await command('plugin_remove',{id:p.id});setRemoveId('')})}>确认移除</button></div>}</article>)}</div><PluginContributionsPanel workspaceId={workspaceId} sessionId={sessionId} revision={revision}/><McpPanel workspaceId={workspaceId}/></Modal>
}



