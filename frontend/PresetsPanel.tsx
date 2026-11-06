import {useEffect,useRef,useState,type FormEvent} from 'react';
import {command,desktop,errorText} from './api';
import {Modal} from './App';
import {BUILTIN_PRESET_ID,KNOWN_APPROVALS,type ModelConfig,type PresetConfig,type PresetInfo,type ProviderInfo} from './types';
const APPROVAL_LABELS:Record<string,string>={ask:'询问（逐次确认）',auto:'自动允许',never:'自动拒绝'};
const blankPreset=():PresetConfig=>({id:crypto.randomUUID(),name:'',description:null,modelId:null,effort:null,approval:null});
export function PresetsPanel({models,providers,onClose,onChanged}:{models:ModelConfig[];providers:ProviderInfo[];onClose:()=>void;onChanged:()=>Promise<void>}) {
 const [presets,setPresets]=useState<PresetInfo[]>([]),[draft,setDraft]=useState<PresetConfig|null>(null);
 const [error,setError]=useState(''),[status,setStatus]=useState(''),[busy,setBusy]=useState(false);
 const alive=useRef(true),locked=useRef(false);
 useEffect(()=>{alive.current=true;void command<PresetInfo[]>('preset_list').then(list=>{if(alive.current)setPresets(list)}).catch(e=>{if(alive.current)setError(errorText(e))});return()=>{alive.current=false}},[]);
 const usableModels=models.filter(m=>m.enabled&&providers.some(p=>p.id===m.providerId&&p.enabled));
 function changed(){setStatus('');setError('')}
 function edit(info:PresetInfo){changed();setDraft({id:info.id,name:info.name,description:info.description,modelId:info.modelId,effort:info.effort,approval:info.approval})}
 function startNew(){changed();setDraft(blankPreset())}
 async function action(fn:()=>Promise<void>){if(locked.current)return;locked.current=true;setBusy(true);setStatus('');setError('');try{await fn()}catch(e){if(alive.current)setError(errorText(e))}finally{locked.current=false;if(alive.current)setBusy(false)}}
 async function refresh(){const list=await command<PresetInfo[]>('preset_list');if(!alive.current)return;setPresets(list);await onChanged()}
 const editing=!!draft&&presets.some(p=>p.id===draft.id);
 async function save(e:FormEvent){e.preventDefault();if(!draft)return;const next=draft;await action(async()=>{
  const stored=await command<PresetConfig>('preset_save',{preset:next});if(!alive.current)return;
  setDraft({...stored});await refresh();if(alive.current)setStatus(`已保存预设 ${stored.name}。`)})}
 async function remove(){if(!draft)return;const id=draft.id;await action(async()=>{
  if(presets.some(p=>p.id===id))await command('preset_remove',{id});if(!alive.current)return;
  setDraft(null);await refresh();if(alive.current)setStatus('已删除预设。')})}
 return <Modal wide title="能力预设" onClose={onClose}><p className="muted">预设把模型、推理档位与审批策略打包成一个名字：发送框一键切换填充，CLI/JSON-RPC/ACP 经 <code>--preset</code> / <code>presetId</code> 引用。未指定的字段不覆盖现有选择。</p>
 {error&&<p role="alert" className="inline-error">{error}</p>}{status&&<p role="status" className="success">{status}</p>}
 <div className="provider-layout">
  <section aria-label="预设目录"><h3>预设</h3>
   <div className="provider-list">{presets.map(info=><div key={info.id} className={`provider-row${draft?.id===info.id?' selected':''}`}><button className="provider-name" onClick={()=>info.id===BUILTIN_PRESET_ID?changed():edit(info)}>{info.name}{info.id===BUILTIN_PRESET_ID&&<em className="builtin-badge">内置</em>}{info.broken&&<em className="off-badge">不可用（{info.broken}）</em>}</button></div>)}</div>
   <button disabled={busy||!desktop()} onClick={startNew}>新建预设</button>
  </section>
  <div className="provider-forms">{draft?<form onSubmit={save} className="settings-form">
   <h3>{editing?'编辑预设':'新建预设'}</h3>
   <label>预设 ID<input required disabled={editing} value={draft.id} onChange={e=>setDraft({...draft,id:e.target.value})}/></label>
   <label>显示名称<input value={draft.name} placeholder="缺省取预设 ID" onChange={e=>setDraft({...draft,name:e.target.value})}/></label>
   <label>说明<input value={draft.description??''} placeholder="一句话说明预设用途（可选）" onChange={e=>setDraft({...draft,description:e.target.value||null})}/></label>
   <label>模型<select value={draft.modelId??''} onChange={e=>setDraft({...draft,modelId:e.target.value||null,effort:null})}><option value="">不指定（沿用发送框当前选择）</option>{usableModels.map(m=><option key={m.id} value={m.id}>{m.name}</option>)}</select></label>
   <label>预设档位<select value={draft.effort??''} disabled={!draft.modelId} onChange={e=>setDraft({...draft,effort:e.target.value||null})}><option value="">跟随模型默认</option>{(models.find(m=>m.id===draft.modelId)?.efforts??[]).map(level=><option key={level} value={level}>{level}</option>)}</select></label>
   <label>审批策略<select value={draft.approval??''} onChange={e=>setDraft({...draft,approval:e.target.value||null})}><option value="">不指定</option>{KNOWN_APPROVALS.map(a=><option key={a} value={a}>{APPROVAL_LABELS[a]}</option>)}</select></label>
   <small>审批策略仅在 CLI / JSON-RPC / ACP 生效；桌面端始终保持交互式审批。</small>
   <div className="actions model-actions">{editing&&<button type="button" className="danger" disabled={busy} onClick={remove}>删除预设</button>}<button className="primary" disabled={busy||!desktop()}>{busy?'处理中…':'保存预设'}</button></div>
  </form>:<p className="muted">选择左侧预设进行编辑，或点击“新建预设”。</p>}</div>
 </div></Modal>
}
