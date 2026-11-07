import {useEffect,useMemo,useState} from 'react';
import {CalendarClock,ChevronDown,Clock,Pencil,Play,Plus,Trash2} from 'lucide-react';
import {command,errorText} from './api';
import {AUTOMATION_KINDS,defaultOnceValue,fmtDateTime,fmtDateTimeSec,fmtLocalInput,fmtSchedule,nextRunAt} from './automations';
import type {AutomationInfo,AutomationKind,SessionInfo} from './types';

export const AUTOMATION_STATUS:{[status:string]:string}={active:'启用中',paused:'已暂停',running:'执行中',completed:'已完成',failed:'失败',missed:'已错过',interrupted:'已中断'};
const RUN_STATUS:{[status:string]:string}={running:'执行中',succeeded:'成功',failed:'失败'};
const WD_LABELS=['一','二','三','四','五','六','日'] as const;

type Draft={title:string;prompt:string;sessionId:string;kind:AutomationKind;at:string;time:string;weekdays:number[];monthDay:number;month:number};
const emptyDraft=():Draft=>({title:'',prompt:'',sessionId:'',kind:'daily',at:defaultOnceValue(),time:'09:30',weekdays:[],monthDay:1,month:1});
const draftOf=(a:AutomationInfo):Draft=>({
  title:a.title,prompt:a.prompt,sessionId:a.sessionId??'',kind:a.kind,
  at:a.at?fmtLocalInput(new Date(a.at)):defaultOnceValue(),
  time:a.time??'09:30',weekdays:a.weekdays??[],monthDay:a.monthDay??1,month:a.month??1,
});
const sessionLabel=(a:AutomationInfo,sessions:SessionInfo[])=>a.sessionId?`会话「${sessions.find(s=>s.id===a.sessionId)?.title??'原会话'}」`:'每次新建会话';
const localTime=(iso:string)=>{const t=Date.parse(iso);return Number.isFinite(t)?fmtDateTime(new Date(t)):iso};
type PanelAction=()=>Promise<unknown>;

/// 侧栏「自动化」面板：周期任务的列表、创建/编辑与运行历史。
export function AutomationsPanel({workspaceId,sessions,onChanged}:{workspaceId:string;sessions:SessionInfo[];onChanged?:()=>void}) {
  const [items,setItems]=useState<AutomationInfo[]>([]);
  const [error,setError]=useState(''),[busy,setBusy]=useState(false),[loaded,setLoaded]=useState(false);
  const [view,setView]=useState<'list'|'form'>('list');
  const [editing,setEditing]=useState<AutomationInfo|null>(null);
  const [draft,setDraft]=useState<Draft>(emptyDraft);
  const [formError,setFormError]=useState('');
  const [removeId,setRemoveId]=useState('');
  async function load(){
    setError('');
    try{
      setItems(await command<AutomationInfo[]>('automation_list',{workspaceId}));
      setLoaded(true);
    }catch(e){setError(errorText(e))}
  }
  useEffect(()=>{void load();const timer=setInterval(()=>void load(),2000);return()=>clearInterval(timer)},[workspaceId]);
  async function action(fn:PanelAction){setBusy(true);setError('');try{await fn();await load();onChanged?.()}catch(e){setError(errorText(e))}finally{setBusy(false)}}
  function openCreate(){setEditing(null);setDraft(emptyDraft());setFormError('');setView('form')}
  function openEdit(a:AutomationInfo){setEditing(a);setDraft(draftOf(a));setFormError('');setView('form')}
  function backToList(){setView('list');setEditing(null);setFormError('')}
  const preview=useMemo(()=>{
    const like={kind:draft.kind,at:draft.kind==='once'&&draft.at?new Date(draft.at).toISOString():null,time:draft.kind==='once'?null:draft.time,weekdays:draft.weekdays,monthDay:draft.monthDay,month:draft.month};
    if(draft.kind==='weekly'&&!draft.weekdays.length)return '请至少选择一个周几';
    if(draft.kind==='once'&&!draft.at)return '填写后显示首次执行时间';
    const t=nextRunAt(like);
    if(!t)return '填写后显示首次执行时间';
    const target=draft.sessionId?`，投递到会话「${sessions.find(s=>s.id===draft.sessionId)?.title??draft.sessionId}」`:'，每次新建会话';
    return `将首先执行于 ${draft.kind==='once'?fmtDateTimeSec(t):fmtDateTime(t)}${target}`;
  },[draft,sessions]);
  function submit(e:React.FormEvent){
    e.preventDefault();
    if(!draft.prompt.trim()){setFormError('请填写任务内容。');return}
    if(draft.kind==='weekly'&&!draft.weekdays.length){setFormError('请至少选择一个周几。');return}
    let at:Date|null=null;
    if(draft.kind==='once'){
      at=draft.at?new Date(draft.at):null;
      if(!at||Number.isNaN(at.getTime())){setFormError('请选择执行时间。');return}
      if(at.getTime()<=Date.now()){setFormError('执行时间必须在未来。');return}
    }
    const params={
      workspaceId,
      title:draft.title.trim(),
      prompt:draft.prompt,
      sessionId:draft.sessionId||null,
      kind:draft.kind,
      at:draft.kind==='once'?(at as Date).toISOString():null,
      time:draft.kind==='once'?null:draft.time,
      weekdays:draft.kind==='weekly'?draft.weekdays:null,
      monthDay:draft.kind==='monthly'||draft.kind==='yearly'?draft.monthDay:null,
      month:draft.kind==='yearly'?draft.month:null,
    };
    void action(async()=>{
      if(editing)await command('automation_update',{id:editing.id,params});
      else await command('automation_create',params);
      backToList();
    });
  }
  const title=view==='list'?'自动化任务':editing?'编辑自动化':'新建自动化';
  return <section role="region" aria-label="自动化任务">
    <div className="auto-toolbar">
      <button type="button" onClick={backToList} disabled={view==='list'}><ChevronDown size={14}/>返回列表</button>
      <button className="primary" type="button" onClick={openCreate} disabled={view==='form'}><Plus size={14}/>新建任务</button>
    </div>
    {error&&<p role="alert" className="inline-error">{error}</p>}
    {view==='list'&&<>
      <p className="auto-note">到期后自动向目标会话发送任务内容并执行一个回合；绑定会话时投递到该会话，未绑定时每次新建会话。应用退出期间到期的任务不补跑，执行仍遵守工具审批。</p>
      <div className="plugin-list">
        {loaded&&!items.length&&<p className="empty-small muted">暂无自动化任务，点击「新建任务」创建。</p>}
        {items.map(a=>{
          const next=Date.parse(a.nextRunAt??'');
          return <article key={a.id} className={a.enabled?undefined:'paused'}>
            <div className="plugin-heading">
              <CalendarClock size={17}/>
              <strong>{a.title||a.prompt.slice(0,20)||'未命名任务'}</strong>
              <em className={a.enabled?'state-badge':'state-badge off'}>{AUTOMATION_STATUS[a.status]??a.status}</em>
              <label className="auto-switch" aria-label="启用或暂停">
                <input type="checkbox" checked={a.enabled} disabled={busy} onChange={e=>void action(()=>command('automation_set_enabled',{id:a.id,enabled:e.target.checked}))}/>
              </label>
            </div>
            <p>{fmtSchedule(a)} · {sessionLabel(a,sessions)} · 已执行 {a.runCount} 次</p>
            <p className="next">{a.enabled?(Number.isFinite(next)?<>下次执行 <b>{fmtDateTime(new Date(next))}</b></>:'等待调度'):'已暂停，不触发执行'}</p>
            {removeId===a.id&&<div className="confirm">确定删除该自动化任务？删除后不再触发。
              <button disabled={busy} onClick={()=>setRemoveId('')}>取消</button>
              <button className="danger" disabled={busy} onClick={()=>{setRemoveId('');void action(()=>command('automation_delete',{id:a.id}))}}>确认删除</button>
            </div>}
            <details className="runs">
              <summary><ChevronDown size={12}/>{a.runs.length?`运行历史（最近 ${a.runs.length} 次）`:'运行历史'}</summary>
              {a.runs.length?<ul className="run-list">{a.runs.slice(0,5).map(r=><li key={r.id}>
                <span className={`run-dot ${r.status==='succeeded'?'ok':r.status==='running'?'run':'fail'}`}/>
                <time>{localTime(r.triggeredAt)}</time>
                <span>{RUN_STATUS[r.status]??r.status}{r.error?`：${r.error}`:''}</span>
                {r.trigger==='manual'&&<span className="run-manual">手动</span>}
                <span className="run-session">{r.sessionId.slice(0,8)}</span>
              </li>)}</ul>:<p className="field-hint">暂无执行记录。</p>}
            </details>
            <div className="plugin-actions">
              <small>{a.id.slice(0,8)}</small>
              <button disabled={busy} onClick={()=>void action(()=>command('automation_run_now',{id:a.id}))}><Play size={12}/>立即执行</button>
              <button disabled={busy} onClick={()=>openEdit(a)}><Pencil size={12}/>编辑</button>
              <button className="danger" disabled={busy} onClick={()=>setRemoveId(a.id)}><Trash2 size={12}/>删除</button>
            </div>
          </article>;
        })}
      </div>
    </>}
    {view==='form'&&<form className="settings-form" onSubmit={submit}>
      <label>标题<input value={draft.title} onChange={e=>setDraft(d=>({...d,title:e.target.value}))} placeholder="例：每日代码评审（可留空）"/></label>
      <label>任务内容<textarea value={draft.prompt} onChange={e=>setDraft(d=>({...d,prompt:e.target.value}))} placeholder="到期时要执行什么，将作为消息发送到目标会话…"/></label>
      <label>目标会话
        <select value={draft.sessionId} onChange={e=>setDraft(d=>({...d,sessionId:e.target.value}))}>
          <option value="">每次新建会话</option>
          {sessions.map(s=><option key={s.id} value={s.id}>{s.title||s.id.slice(0,8)}</option>)}
        </select>
        <span className="field-hint">仅列出当前工作区的会话；绑定的会话正忙时，任务将排队等待当前回合结束。</span>
      </label>
      <label>执行周期
        <select value={draft.kind} onChange={e=>setDraft(d=>({...d,kind:e.target.value as AutomationKind}))}>
          {AUTOMATION_KINDS.map(k=><option key={k.value} value={k.value}>{k.label}</option>)}
        </select>
      </label>
      {draft.kind==='once'&&<div>
        <label>执行时间（含秒，24 小时制）<input type="datetime-local" step={1} value={draft.at} onChange={e=>setDraft(d=>({...d,at:e.target.value}))}/></label>
        <span className="field-hint">默认为今天当前时刻、秒归零；应用退出期间错过则标记为已错过，不补跑。</span>
      </div>}
      {draft.kind!=='once'&&<label>时间（24 小时制）<input type="time" value={draft.time} onChange={e=>setDraft(d=>({...d,time:e.target.value}))}/></label>}
      {draft.kind==='weekly'&&<div><span className="form-label-inline">周几</span>
        <div className="wd-row">{WD_LABELS.map((label,i)=><label key={label} className="wd-item">
          <input type="checkbox" checked={draft.weekdays.includes(i+1)} onChange={e=>setDraft(d=>({...d,weekdays:e.target.checked?[...d.weekdays,i+1].sort((x,y)=>x-y):d.weekdays.filter(x=>x!==i+1)}))}/>
          {label}
        </label>)}</div>
      </div>}
      {draft.kind==='monthly'&&<label>几号<input type="number" min={1} max={31} value={draft.monthDay} onChange={e=>setDraft(d=>({...d,monthDay:Number(e.target.value)}))}/>
        <span className="field-hint">月份中没有该日期时（如 2 月无 31 日）跳过该月。</span>
      </label>}
      {draft.kind==='yearly'&&<>
        <div className="pair-row">
          <label>月份<select value={draft.month} onChange={e=>setDraft(d=>({...d,month:Number(e.target.value)}))}>{Array.from({length:12},(_,i)=><option key={i+1} value={i+1}>{i+1} 月</option>)}</select></label>
          <label>日期<input type="number" min={1} max={31} value={draft.monthDay} onChange={e=>setDraft(d=>({...d,monthDay:Number(e.target.value)}))}/></label>
        </div>
        <span className="field-hint">平年没有 2 月 29 日时跳过该年。</span>
      </>}
      <div className="preview-line" aria-live="polite"><Clock size={13}/><span>{preview}</span></div>
      {formError&&<div className="inline-error" role="alert">{formError}</div>}
      <div className="actions">
        <button type="button" onClick={backToList}>取消</button>
        <button className="primary" type="submit" disabled={busy}>{editing?'保存修改':'创建任务'}</button>
      </div>
    </form>}
    <p className="muted auto-count">{loaded?`${items.length} / 1000`:'加载中…'}</p>
  </section>;
}
