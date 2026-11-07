import {useEffect,useRef,useState} from 'react';
import {AlertTriangle,Check,ChevronDown,FileMinus,FilePen,FilePlus,RefreshCw} from 'lucide-react';
import {command,errorText} from './api';
import {DiffView,diffStats,parseUnifiedDiff} from './DiffView';
import type {FileChange,WorkspaceFileDiff} from './types';
type DiffState={loading?:boolean;view?:WorkspaceFileDiff;error?:string};
const KIND_ICONS:{[kind:string]:typeof FilePen}={modified:FilePen,added:FilePlus,deleted:FileMinus};
// 改动审阅视图：自 DiffPanel 迁入右侧面板；diff 懒加载，打开期间轮询保留用户勾选。
export function ChangesView({workspaceId}:{workspaceId:string}) {
 const [changes,setChanges]=useState<FileChange[]>([]),[selected,setSelected]=useState<string[]>([]),[error,setError]=useState(''),[busy,setBusy]=useState(false),[confirm,setConfirm]=useState(false),[loaded,setLoaded]=useState(false),[status,setStatus]=useState('');
 const [expanded,setExpanded]=useState<Record<string,boolean>>({}),[diffs,setDiffs]=useState<Record<string,DiffState>>({});
 const changesRef=useRef<FileChange[]>([]),expandedRef=useRef<Record<string,boolean>>({});
 expandedRef.current=expanded;
 async function load(resetSelection:boolean) {
  setError('');
  try{
   const list=await command<FileChange[]>('workspace_diff',{workspaceId});
   const oldPaths=new Set(changesRef.current.map(c=>c.path));
   setChanges(list);changesRef.current=list;setLoaded(true);
   if(resetSelection){
    setSelected(list.filter(c=>!c.conflict).map(c=>c.path));setConfirm(false);
    // 清空差异缓存后，对保持展开的文件自动重算，避免展开区悬空。
    setDiffs({});
    for(const path of Object.keys(expandedRef.current))if(expandedRef.current[path])void loadDiff(path);
   }
   else setSelected(prev=>[...prev.filter(p=>list.some(c=>c.path===p)),...list.filter(c=>!c.conflict&&!oldPaths.has(c.path)).map(c=>c.path)]);
  }catch(e){setError(errorText(e))}
 }
 useEffect(()=>{void load(true);const timer=setInterval(()=>{void load(false)},5000);return()=>clearInterval(timer)},[workspaceId]);
 async function apply() {
  setBusy(true);setError('');
  try{
   await command('workspace_apply',{workspaceId,paths:selected,reviewed:changes.filter(change=>selected.includes(change.path))});
   setConfirm(false);setStatus('已将选中更改同步到原项目。');await load(true);
  }catch(e){setError(errorText(e));setConfirm(false)}finally{setBusy(false)}
 }
 function toggleExpand(path:string) {
  setExpanded(old=>({...old,[path]:!old[path]}));
  if(!diffs[path])void loadDiff(path);
 }
 async function loadDiff(path:string) {
  setDiffs(old=>({...old,[path]:{loading:true}}));
  try{const view=await command<WorkspaceFileDiff>('workspace_file_diff',{workspaceId,path});setDiffs(old=>({...old,[path]:{view}}))}
  catch(e){setDiffs(old=>({...old,[path]:{error:errorText(e)}}))}
 }
 return <>
  <div className="changes-bar"><p>对比托管工作副本与原项目。展开文件查看行级差异，勾选后同步。</p><button type="button" className="soft" disabled={busy} onClick={()=>void load(true)}><RefreshCw size={14}/>刷新</button></div>
  {error&&<p role="alert" className="inline-error">{error}</p>}
  {status&&<p role="status" className="success">{status}</p>}
  <div className="changes-list">
   {loaded&&!changes.length&&<div className="diff-empty"><Check size={26}/><h3>工作副本没有待同步更改</h3><p className="muted">下一次改进，从一个想法开始。</p></div>}
   {changes.map(c=>{
    const state=diffs[c.path],stats=state?.view?.unified?diffStats(parseUnifiedDiff(state.view.unified)):null;
    const Icon=KIND_ICONS[c.kind]??FilePen;
    return <article className="diff-file" key={c.path}>
     <div className="diff-file-row">
      <input type="checkbox" checked={selected.includes(c.path)} disabled={c.conflict||busy} aria-label={`选择 ${c.path}`} onChange={e=>setSelected(old=>e.target.checked?[...old,c.path]:old.filter(p=>p!==c.path))}/>
      <button type="button" className="diff-file-toggle" aria-expanded={!!expanded[c.path]} onClick={()=>toggleExpand(c.path)}>
       <ChevronDown size={14} className="chev"/><Icon size={14}/>
       <span className="diff-path">{c.path}</span>
      </button>
      {stats&&(stats.add>0||stats.del>0)&&<span className="diff-file-stat">{stats.add>0&&<b className="stat-add">+{stats.add}</b>}{stats.del>0&&<b className="stat-del">−{stats.del}</b>}</span>}
      {c.conflict?<span className="conflict-chip">冲突</span>:<em className={`change-kind`}>{c.kind}</em>}
     </div>
     {expanded[c.path]&&<>
      {c.conflict&&<div className="conflict-note"><AlertTriangle size={14}/>原项目在同步前也发生了变更，需先处理冲突才能同步此文件。</div>}
      {state?.loading?<p className="muted diff-loading">正在计算差异…</p>
       :state?.error?<p role="alert" className="inline-error">{state.error}</p>
       :state?.view?.unified?<DiffView diff={state.view.unified}/>
       :state?.view?<p className="muted diff-loading">二进制文件或文本超过 1 MiB，不渲染行级差异。</p>:null}
     </>}
    </article>;
   })}
  </div>
  <div className="changes-foot">
   <span className="muted">已选择 {selected.length} 个文件</span>
   {confirm?<><span>确认写入原项目？</span><button onClick={()=>setConfirm(false)} disabled={busy}>返回审阅</button><button className="primary" onClick={apply} disabled={busy}>{busy?'同步中…':'确认应用更改'}</button></>
    :<button className="primary" disabled={busy||!selected.length} onClick={()=>setConfirm(true)}>同步选中文件</button>}
  </div>
 </>;
}
