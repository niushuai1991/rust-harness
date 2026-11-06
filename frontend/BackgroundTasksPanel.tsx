import {useEffect,useRef,useState} from 'react';
import {CalendarClock,RefreshCw,Terminal,X} from 'lucide-react';
import {command,errorText} from './api';
import type {JobInfo,ScheduleInfo} from './types';

export const JOB_STATUS:{[status:string]:string}={running:'运行中',completed:'已完成',failed:'失败',cancelled:'已取消',interrupted:'已中断'};
export const SCHEDULE_STATUS:{[status:string]:string}={pending:'等待执行',running:'执行中',completed:'已完成',failed:'失败',missed:'已错过',interrupted:'已中断'};
const localTime=(iso:string)=>{const t=Date.parse(iso);return Number.isFinite(t)?new Date(t).toLocaleString():iso};
type PanelAction=()=>Promise<unknown>;

/// 顶栏“后台任务”按钮下方的浮层卡片；点开关按钮、点外部或按 Escape 收起。
export function BackgroundTasksPanel({sessionId,onClose}:{sessionId:string;onClose:()=>void}) {
 const [jobs,setJobs]=useState<JobInfo[]>([]),[schedules,setSchedules]=useState<ScheduleInfo[]>([]);
 const [error,setError]=useState(''),[busy,setBusy]=useState(false),[loaded,setLoaded]=useState(false),[removeId,setRemoveId]=useState('');
 const root=useRef<HTMLElement>(null);
 async function load(){
  setError('');
  try{
   const [j,s]=await Promise.all([command<JobInfo[]>('job_list',{sessionId}),command<ScheduleInfo[]>('schedule_list',{sessionId})]);
   setJobs(j);setSchedules(s);setLoaded(true);
  }catch(e){setError(errorText(e))}
 }
 useEffect(()=>{void load();const timer=setInterval(()=>void load(),2000);return()=>clearInterval(timer)},[sessionId]);
 useEffect(()=>{
  // 开关按钮的收起由 App 的 toggle 负责，这里只处理点外部与 Escape。
  function outside(e:MouseEvent){
   const t=e.target as Node;
   if(root.current?.contains(t))return;
   if(t instanceof Element&&t.closest('[aria-controls="task-popover"]'))return;
   onClose();
  }
  function key(e:KeyboardEvent){if(e.key==='Escape')onClose()}
  document.addEventListener('mousedown',outside);
  document.addEventListener('keydown',key);
  return()=>{document.removeEventListener('mousedown',outside);document.removeEventListener('keydown',key)};
 },[onClose]);
 async function action(fn:PanelAction){setBusy(true);setError('');try{await fn();await load()}catch(e){setError(errorText(e))}finally{setBusy(false)}}
 return <section ref={root} id="task-popover" className="task-popover" role="region" aria-label="后台任务">
  <header>
   <h2>后台任务</h2>
   <div className="task-popover-tools">
    <button className="icon-button" aria-label="刷新" disabled={busy} onClick={()=>void load()}><RefreshCw size={15}/></button>
    <button className="icon-button" aria-label="收起面板" onClick={onClose}><X size={17}/></button>
   </div>
  </header>
  <p className="muted task-popover-note">后台 Shell 在托管副本中执行并占用工作区锁；定时任务过期不补跑，执行仍遵守工具审批。展开期间每 2 秒自动刷新；宿主重启后的后台任务标记为已中断且不重放。</p>
  {error&&<p role="alert" className="inline-error">{error}</p>}
  <div className="section-label">后台 Shell 任务</div>
  <div className="plugin-list">{loaded&&!jobs.length&&<p className="empty-small muted">本会话暂无后台任务。</p>}
   {jobs.map(j=><article key={j.id}>
    <div className="plugin-heading"><Terminal size={17}/><strong>{j.command}</strong><small>{j.status==='running'&&j.cancelRequested?'已请求取消':JOB_STATUS[j.status]??j.status}</small></div>
    <p className="muted">{localTime(j.createdAt)} · 任务 {j.id}</p>
    {j.status==='running'&&<div className="plugin-actions"><small>{j.cancelRequested?'等待进程退出…':'在托管副本中运行'}</small><button className="danger" disabled={busy||j.cancelRequested} onClick={()=>action(()=>command('job_cancel',{sessionId,jobId:j.id}))}>取消任务</button></div>}
   </article>)}
  </div>
  <div className="section-label">定时任务</div>
  <div className="plugin-list">{loaded&&!schedules.length&&<p className="empty-small muted">本会话暂无定时任务。</p>}
   {schedules.map(s=><article key={s.id}>
    <div className="plugin-heading"><CalendarClock size={17}/><strong>{s.task}</strong><small>{SCHEDULE_STATUS[s.status]??s.status}</small></div>
    <p className="muted">{localTime(s.at)} · 模型 {s.modelId}</p>
    {s.status==='pending'&&(removeId===s.id?<div className="confirm">确定删除该定时任务？<button disabled={busy} onClick={()=>setRemoveId('')}>取消</button><button className="danger" disabled={busy} onClick={()=>action(async()=>{await command('schedule_delete',{sessionId,id:s.id});setRemoveId('')})}>确认删除</button></div>:<div className="plugin-actions"><small>{s.id}</small><button className="danger" disabled={busy} onClick={()=>setRemoveId(s.id)}>删除任务</button></div>)}
   </article>)}
  </div>
 </section>;
}
