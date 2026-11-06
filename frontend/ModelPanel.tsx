import {useEffect,useRef,useState,type FormEvent} from 'react';
import {Pencil,Plus,RefreshCw,Trash2} from 'lucide-react';
import {command,desktop,errorText} from './api';
import {Modal} from './App';
import {KNOWN_EFFORTS,type ApiMode,type ModelConfig,type ProviderInfo,type ProviderUsageSnapshot,type UsageCardKind} from './types';
const modes:{id:ApiMode;name:string}[]=[{id:'chatCompletions',name:'Chat Completions'},{id:'responses',name:'Responses'},{id:'anthropic',name:'Anthropic'}];
const blankProvider=():ProviderInfo=>({id:crypto.randomUUID(),name:'自定义供应商',builtin:false,compatibility:'standard',endpoints:modes.map(m=>({apiMode:m.id,baseUrl:''})),defaultModel:'',credentialRef:'',proxy:null,enabled:true,builtinModels:[],disabledModels:[]});
const blankModel=(p:ProviderInfo):ModelConfig=>({id:crypto.randomUUID(),name:'',providerId:p.id,compatibility:p.compatibility,apiMode:p.endpoints[0]?.apiMode??'chatCompletions',baseUrl:p.endpoints[0]?.baseUrl??'',model:p.defaultModel,credentialRef:p.credentialRef,proxy:p.proxy,maxOutputTokens:null,efforts:[],effort:null,enabled:true,builtin:false});
const snapshot=(p:ProviderInfo)=>JSON.stringify(p);
const snapshotModel=(m:ModelConfig)=>JSON.stringify(m);
const USAGE_CARD_LABELS:Record<UsageCardKind,string>={fiveHour:'5 小时剩余',weekly:'每周剩余',tools:'工具调用'};
function formatResetTime(resetAt:number|null,kind:UsageCardKind):string{
 if(resetAt==null)return'';
 const date=new Date(resetAt),suffix=' 重置';
 if(kind==='fiveHour'){const now=new Date();const sameDay=date.getFullYear()===now.getFullYear()&&date.getMonth()===now.getMonth()&&date.getDate()===now.getDate();
  if(sameDay)return`${String(date.getHours()).padStart(2,'0')}:${String(date.getMinutes()).padStart(2,'0')}${suffix}`;}
 return`${date.getMonth()+1}月${date.getDate()}日${suffix}`;
}
export function ModelPanel({models,onClose,onSaved}:{models:ModelConfig[];onClose:()=>void;onSaved:()=>Promise<void>}) {
 const [providers,setProviders]=useState<ProviderInfo[]>([]),[provider,setProvider]=useState<ProviderInfo|null>(null);
 const [draft,setDraft]=useState<ModelConfig|null>(null),[viewBuiltin,setViewBuiltin]=useState<string|null>(null);
 const [secret,setSecret]=useState(''),[error,setError]=useState(''),[status,setStatus]=useState(''),[busy,setBusy]=useState(false);
 const [fetched,setFetched]=useState<string[]>([]);
 const [savedProvider,setSavedProvider]=useState(''),[savedModel,setSavedModel]=useState('');
 const [usage,setUsage]=useState<ProviderUsageSnapshot|null>(null),[usageError,setUsageError]=useState(''),[usageBusy,setUsageBusy]=useState(false);
 const originals=useRef<ProviderInfo[]>([]),revision=useRef(0),alive=useRef(true),locked=useRef(false);
 const usageRevision=useRef(0),usageLoadedAt=useRef(0),usageInflight=useRef(false);
 useEffect(()=>{alive.current=true;void command<ProviderInfo[]>('provider_list').then(list=>{if(!alive.current)return;originals.current=list;setProviders(list);if(list[0])choose(list[0])}).catch(e=>{if(alive.current)setError(errorText(e))});return()=>{alive.current=false;revision.current++}},[]);
 const codingPlan=!!provider&&provider.builtin&&provider.compatibility==='bigModel';
 useEffect(()=>{
  if(!codingPlan||!provider?.credentialRef){setUsage(null);setUsageError('');usageLoadedAt.current=0;return}
  void loadUsage();
  // eslint-disable-next-line react-hooks/exhaustive-deps -- 按供应商与凭据引用变化触发；loadUsage 读取最新 provider。
 },[codingPlan,provider?.id,provider?.credentialRef]);
 function changed(){revision.current++;setStatus('');setError('')}
 function choose(p:ProviderInfo){changed();const fresh=structuredClone(p);setProvider(fresh);setDraft(null);setViewBuiltin(null);setFetched([]);setSecret('');setUsage(null);setUsageError('');usageLoadedAt.current=0;setSavedProvider(snapshot(originals.current.find(x=>x.id===p.id)??p));setSavedModel('')}
 function editProvider(next:ProviderInfo){changed();setProvider(next)}
 function editModel(next:ModelConfig){changed();setDraft(next)}
 function startModel(){if(!provider)return;changed();setViewBuiltin(null);setFetched([]);const fresh=blankModel(provider);setDraft(fresh);setSavedModel('')}
 function startEditModel(m:ModelConfig){changed();setViewBuiltin(null);setFetched([]);setDraft(structuredClone(m));setSavedModel(snapshotModel(m))}
 function viewBuiltinModel(id:string){changed();setDraft(null);setFetched([]);setViewBuiltin(id)}
 async function action(fn:(token:number)=>Promise<void>){if(locked.current)return;locked.current=true;const token=revision.current;setBusy(true);setStatus('');setError('');try{await fn(token)}catch(e){if(alive.current&&token===revision.current)setError(errorText(e))}finally{locked.current=false;if(alive.current)setBusy(false)}}
 const current=(token:number)=>alive.current&&token===revision.current;
 async function loadUsage(force=false){
  if(!provider||usageInflight.current)return;
  if(!force&&usageLoadedAt.current&&Date.now()-usageLoadedAt.current<60_000)return;
  const token=++usageRevision.current;usageInflight.current=true;
  setUsageBusy(true);setUsageError('');
  try{const snap=await command<ProviderUsageSnapshot>('provider_usage',{providerId:provider.id});
   if(!alive.current||token!==usageRevision.current)return;
   setUsage(snap);usageLoadedAt.current=Date.now();
  }catch(e){if(!alive.current||token!==usageRevision.current)return;setUsage(null);setUsageError(errorText(e))}
  finally{usageInflight.current=false;if(alive.current&&token===usageRevision.current)setUsageBusy(false)}
 }
 async function refreshLists(){await onSaved();if(!alive.current)return;const list=await command<ProviderInfo[]>('provider_list');if(!alive.current)return;originals.current=list;setProviders(list)}
 async function persistProvider(e:FormEvent){e.preventDefault();if(!provider)return;const p=provider,key=secret;await action(async token=>{
  // Detect external credential/proxy changes before writing a stale supplier draft.
  const fresh=await command<ProviderInfo[]>('provider_list');const previous=originals.current.find(x=>x.id===p.id),latest=fresh.find(x=>x.id===p.id);
  if(previous&&(!latest||latest.credentialRef!==previous.credentialRef||latest.proxy!==previous.proxy))throw new Error('供应商凭据或代理已在其他位置更改，请关闭并重新打开设置后保存。');
  if(!current(token))return;
  const stored=await command<ProviderInfo>('provider_save',{config:{...p,defaultModel:p.builtin?p.defaultModel:(p.defaultModel.trim()||draft?.model||'')},secret:key||undefined});
  if(!current(token))return;
  originals.current=[...originals.current.filter(x=>x.id!==stored.id),stored];
  setProvider(structuredClone(stored));setSecret('');setSavedProvider(snapshot(stored));
  await refreshLists();if(current(token))setStatus('已保存供应商；密钥由系统凭据管理器保管。');
  if(codingPlan)void loadUsage(true);
 })}
 async function saveModel(e:FormEvent){e.preventDefault();if(!draft||!provider)return;const m=draft;await action(async token=>{
  const next={...m,providerId:provider.id,compatibility:provider.compatibility,baseUrl:provider.endpoints.find(x=>x.apiMode===m.apiMode)?.baseUrl??m.baseUrl,credentialRef:provider.credentialRef,proxy:provider.proxy};
  await command('model_save',{config:next});if(!current(token))return;
  setDraft(structuredClone(next));setSavedModel(snapshotModel(next));await refreshLists();if(current(token)){setStatus('已保存模型。');setDraft(null);setSavedModel('')}
 })}
 async function removeModel(m:ModelConfig){await action(async token=>{await command('model_remove',{modelId:m.id});if(!current(token))return;if(draft?.id===m.id){setDraft(null);setSavedModel('')}await refreshLists()})}
 async function toggleProvider(p:ProviderInfo,enabled:boolean){await action(async token=>{const stored=await command<ProviderInfo>('provider_save',{config:{...p,enabled},secret:undefined});if(!current(token))return;originals.current=[...originals.current.filter(x=>x.id!==stored.id),stored];if(provider?.id===stored.id){setProvider(structuredClone(stored));setSavedProvider(snapshot(stored))}await refreshLists()})}
 async function toggleBuiltinModel(p:ProviderInfo,modelId:string,enabled:boolean){await action(async token=>{const disabled=p.disabledModels.filter(id=>id!==modelId);if(!enabled)disabled.push(modelId);const stored=await command<ProviderInfo>('provider_save',{config:{...p,disabledModels:disabled},secret:undefined});if(!current(token))return;originals.current=[...originals.current.filter(x=>x.id!==stored.id),stored];if(provider?.id===stored.id)setProvider(structuredClone(stored));await refreshLists()})}
 async function toggleModel(m:ModelConfig,enabled:boolean){await action(async token=>{await command('model_save',{config:{...m,enabled}});if(!current(token))return;await refreshLists()})}
 async function copyProvider(){if(!provider)return;await action(async token=>{const p=await command<ProviderInfo>('provider_copy',{id:provider.id});if(!current(token))return;originals.current=[...originals.current,p];setProviders(list=>[...list,p]);choose(p)})}
 async function removeProvider(){if(!provider)return;const id=provider.id;await action(async token=>{if(originals.current.some(p=>p.id===id))await command('provider_remove',{id});if(!current(token))return;originals.current=originals.current.filter(p=>p.id!==id);const list=providers.filter(p=>p.id!==id);setProviders(list);if(list[0])choose(list[0]);else{setProvider(null);setDraft(null)}})}
 async function fetchAvailable(){if(!provider||!draft)return;await action(async token=>{const ids=await command<string[]>('model_fetch_available',{providerId:provider.id,apiMode:draft.apiMode});if(current(token))setFetched(ids)})}
 const providerDirty=!!provider&&snapshot(provider)!==savedProvider;
 const modelClean=!!draft&&snapshotModel(draft)===savedModel;
 const savedDraft=!!draft&&models.some(m=>m.id===draft.id);
 const builtinView=provider?.builtinModels.find(m=>m.id===viewBuiltin);
 const currentModels=provider?models.filter(m=>m.providerId===provider.id):[];
 const providerRow=(p:ProviderInfo)=><div key={p.id} className={`provider-row${p.id===provider?.id?' selected':''}`}><button className="provider-name" onClick={()=>choose(p)}>{p.name}{p.builtin&&<em className="builtin-badge">内置</em>}{!p.enabled&&<em className="off-badge">已停用</em>}</button><label className="switch" title={p.enabled?'停用供应商':'启用供应商'}><input type="checkbox" aria-label={`启用供应商 ${p.name}`} checked={p.enabled} disabled={busy} onChange={e=>toggleProvider(p,e.target.checked)}/></label></div>;
 const usageSection=!codingPlan?null:<section className="plan-card" aria-label="套餐与剩余额度">
  <div className="plan-card-top"><strong>{usage?.planName??'智谱 Coding Plan'}</strong></div>
  <div className="plan-divider"/>
  <p className="plan-label">剩余额度</p>
  {!provider.credentialRef&&<p className="muted">配置 API 密钥后可查看剩余额度。</p>}
  {provider.credentialRef&&(usageError?<div className="usage-error" role="status">{usageError}<button type="button" disabled={usageBusy} onClick={()=>void loadUsage(true)}>重试</button></div>
   :usageBusy&&!usage?<p className="muted">正在获取额度…</p>
   :usage&&usage.cards.length?<div className="usage-cards">{usage.cards.map(card=>{const pct=card.remainingPercent==null?null:Math.max(0,Math.min(100,Math.round(card.remainingPercent)));return <div key={card.kind} className="usage-card">
    <p className="usage-card-label">{USAGE_CARD_LABELS[card.kind]}</p>
    <div className="usage-card-value"><strong>{pct==null?'—':`${pct}%`}</strong><span>{formatResetTime(card.resetAt,card.kind)}</span></div>
    <div className="usage-bar" role="progressbar" aria-valuenow={pct??0} aria-valuemin={0} aria-valuemax={100} aria-label={`${USAGE_CARD_LABELS[card.kind]}剩余百分比`}><i className={pct!=null&&pct<20?'warn':''} style={{width:`${pct??0}%`}}/></div>
   </div>})}</div>
   :usage&&!usage.cards.length?<p className="muted">当前套餐未返回额度明细。</p>:null)}
 </section>;
 return <Modal wide title="模型设置" onClose={onClose}><p className="muted">先选择供应商并填写密钥，再在该供应商下添加或启用模型；发送框按“供应商 → 模型 → 档位”选择。密钥仅存入系统凭据管理器。</p>
 {error&&<p role="alert" className="inline-error">{error}</p>}{status&&<p role="status" className="success">{status}</p>}
 <div className="provider-layout"><nav className="provider-nav" aria-label="供应商目录">
  <h3 className="nav-group-title">内置供应商</h3><div className="provider-list">{providers.filter(p=>p.builtin).map(providerRow)}</div>
  <h3 className="nav-group-title">自定义供应商</h3><div className="provider-list">{providers.filter(p=>!p.builtin).map(providerRow)}</div>
  <button className="nav-add" disabled={busy||!desktop()} onClick={()=>{const p=blankProvider();setProviders(list=>[...list,p]);choose(p)}}>添加自定义供应商</button>
 </nav>
 {provider?<div className="provider-detail">
  <div className="detail-head"><h3>{provider.name}</h3>{provider.builtin&&<em className="builtin-badge">内置</em>}{!provider.enabled&&<em className="off-badge">已停用</em>}{codingPlan&&<button type="button" className="icon-soft" title="刷新额度" aria-label="刷新额度" disabled={usageBusy} onClick={()=>void loadUsage(true)}><RefreshCw size={14}/></button>}</div>
  {usageSection}
  <form onSubmit={persistProvider} className="settings-form"><h4>连接设置</h4>
  <label>供应商名称<input required readOnly={provider.builtin} value={provider.name} onChange={e=>editProvider({...provider,name:e.target.value})}/></label>
  <label>API 密钥<input type="password" autoComplete="new-password" value={secret} placeholder={provider.credentialRef?'留空保留已保存密钥':'输入此供应商的密钥'} onChange={e=>{changed();setSecret(e.target.value)}}/></label>
  <label>HTTP 代理（可选）<input value={provider.proxy??''} placeholder="http://127.0.0.1:7890" onChange={e=>editProvider({...provider,proxy:e.target.value||null})}/></label>
  {provider.builtin&&<small>内置供应商名称与端点固定，可启用/停用或复制后修改。</small>}
  <div className="provider-actions"><button type="button" disabled={busy||!originals.current.some(p=>p.id===provider.id)} onClick={copyProvider}>复制供应商</button>{!provider.builtin&&<button type="button" className="danger" disabled={busy||models.some(m=>m.providerId===provider.id)} onClick={removeProvider}>删除供应商</button>}</div>
  {!provider.builtin&&models.some(m=>m.providerId===provider.id)&&<small>删除供应商前，请先逐一删除关联的模型。</small>}
  {!provider.builtin&&<label>默认模型<input value={provider.defaultModel} onChange={e=>editProvider({...provider,defaultModel:e.target.value})}/></label>}
  <div className="actions"><button className="primary" disabled={busy||!desktop()||(!providerDirty&&!secret)}>{busy?'处理中…':'保存供应商'}</button></div>
  </form>
  <section className="model-section" aria-label="模型列表">
   <div className="section-head"><h4>模型列表</h4><button type="button" className="soft" disabled={busy||!provider.enabled||!desktop()} onClick={startModel}><Plus size={13}/>添加模型</button></div>
   <div className="model-list">
   {provider.builtinModels.map(m=>{const enabled=provider.enabled&&!provider.disabledModels.includes(m.id);return <div key={m.id} className={`model-row${viewBuiltin===m.id?' selected':''}`}><button className="model-name" onClick={()=>viewBuiltinModel(m.id)}>{m.name}</button><em className="builtin-badge">内置</em>{!!m.efforts.length&&<em className="chip">{m.efforts.join(' / ')}</em>}{!enabled&&<em className="off-badge">已停用</em>}<span className="model-row-actions"><button type="button" className="icon-soft" aria-label={`查看 ${m.name}`} onClick={()=>viewBuiltinModel(m.id)}><Pencil size={13}/></button><label className="switch" title={enabled?'停用内置模型':'启用内置模型'}><input type="checkbox" aria-label={`启用模型 ${m.name}`} checked={enabled} disabled={busy||!provider.enabled} onChange={e=>toggleBuiltinModel(provider,m.id,e.target.checked)}/></label></span></div>})}
   {currentModels.map(m=><div key={m.id} className={`model-row${draft?.id===m.id?' selected':''}`}><button className="model-name" onClick={()=>startEditModel(m)}>{m.name} · {m.model}</button>{!m.enabled&&<em className="off-badge">已停用</em>}<span className="model-row-actions"><button type="button" className="icon-soft" aria-label={`编辑 ${m.name}`} onClick={()=>startEditModel(m)}><Pencil size={13}/></button><button type="button" className="icon-soft" aria-label={`删除模型 ${m.name}`} disabled={busy} onClick={()=>void removeModel(m)}><Trash2 size={13}/></button><label className="switch" title={m.enabled?'停用模型':'启用模型'}><input type="checkbox" aria-label={`启用模型 ${m.name}`} checked={m.enabled} disabled={busy} onChange={e=>toggleModel(m,e.target.checked)}/></label></span></div>)}
   {!provider.builtinModels.length&&!currentModels.length&&<p className="muted model-list-empty">该供应商还没有模型，点击“添加模型”。</p>}
   </div>
  </section>
 </div>:<p className="muted">{busy?'处理中…':'正在加载供应商…'}</p>}</div>
 {provider&&draft&&<Modal title={savedDraft?'编辑模型':'添加模型'} onClose={()=>{setDraft(null);setFetched([])}}><form onSubmit={saveModel} className="settings-form">
  <label>显示名称<input required value={draft.name} onChange={e=>editModel({...draft,name:e.target.value})}/></label>
  <label>协议<select value={draft.apiMode} onChange={e=>{const apiMode=e.target.value as ApiMode;if(!provider.endpoints.some(x=>x.apiMode===apiMode))editProvider({...provider,endpoints:[...provider.endpoints,{apiMode,baseUrl:''}]});editModel({...draft,apiMode})}}>{modes.filter(m=>!provider.builtin||provider.endpoints.some(x=>x.apiMode===m.id)).map(m=><option key={m.id} value={m.id}>{m.name}</option>)}</select></label>
  {!provider.builtin&&<label>API 地址<input required type="url" value={provider.endpoints.find(x=>x.apiMode===draft.apiMode)?.baseUrl??''} onChange={e=>editProvider({...provider,endpoints:provider.endpoints.map(x=>x.apiMode===draft.apiMode?{...x,baseUrl:e.target.value}:x)})}/></label>}
  <label>模型标识<input required list="fetched-model-ids" value={draft.model} onChange={e=>editModel({...draft,model:e.target.value})}/></label>
  <datalist id="fetched-model-ids">{fetched.map(id=><option key={id} value={id}/>)}</datalist>
  <button type="button" className="soft" disabled={busy||!desktop()} onClick={()=>void fetchAvailable()}>{fetched.length?'重新获取模型列表':'从 API 获取模型列表'}</button>
  {fetched.length>0&&<label>从检测结果选择<select value="" onChange={e=>{if(e.target.value)editModel({...draft,model:e.target.value})}}><option value="" disabled>{fetched.length} 个候选模型</option>{fetched.map(id=><option key={id} value={id}>{id}</option>)}</select></label>}
  <fieldset className="effort-fieldset"><legend>支持的推理档位（可多选）</legend>{KNOWN_EFFORTS.map(level=><label key={level} className="effort-option"><input type="checkbox" aria-label={`档位 ${level}`} checked={draft.efforts.includes(level)} onChange={e=>{const efforts=e.target.checked?[...draft.efforts,level]:draft.efforts.filter(x=>x!==level);editModel({...draft,efforts,effort:efforts.includes(draft.effort??'')?draft.effort:null})}}/>{level}</label>)}</fieldset>
  {!!draft.efforts.length&&<label>默认推理档位<select value={draft.effort??''} onChange={e=>editModel({...draft,effort:e.target.value||null})}><option value="">跟随供应商默认</option>{draft.efforts.map(level=><option key={level} value={level}>{level}</option>)}</select></label>}
  <label>最大输出 Token（可选）<input type="number" min="1" max="2147483647" step="1" value={draft.maxOutputTokens??''} onChange={e=>editModel({...draft,maxOutputTokens:e.target.value?Number(e.target.value):null})}/></label>
  <p className="muted">连接测试会向已保存模型发送少量请求并消耗额度。修改配置后需先保存。</p>
  <div className="actions model-actions"><button type="button" disabled={busy||!savedDraft||!modelClean} onClick={()=>action(async token=>{await command<string>('model_test',{modelId:draft.id});if(current(token))setStatus('模型连接成功。')})}>测试已保存配置</button><button className="primary" disabled={busy||!desktop()}>{busy?'处理中…':'保存模型'}</button></div>
 </form></Modal>}
 {builtinView&&<Modal title={`内置模型：${builtinView.name}`} onClose={()=>setViewBuiltin(null)}><section aria-label="内置模型信息" className="settings-form"><p>模型标识 <code>{builtinView.id}</code>（内置目录成员，不可修改或删除，可启用/停用）。</p><p>支持档位：{builtinView.efforts.length?builtinView.efforts.join(' / '):'无'}；协议固定为 Anthropic 端点。</p><p className="muted">启用后即可在发送框的模型列表中选择。</p></section></Modal>}
 </Modal>
}
