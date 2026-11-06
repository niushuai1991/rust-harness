import {useEffect,useRef,useState,type FormEvent} from 'react';
import {command,desktop,errorText} from './api';
import {Modal} from './App';
import {KNOWN_EFFORTS,type ApiMode,type ModelConfig,type ProviderInfo} from './types';
const modes:{id:ApiMode;name:string}[]=[{id:'chatCompletions',name:'Chat Completions'},{id:'responses',name:'Responses'},{id:'anthropic',name:'Anthropic'}];
const blankProvider=():ProviderInfo=>({id:crypto.randomUUID(),name:'自定义供应商',builtin:false,compatibility:'standard',endpoints:modes.map(m=>({apiMode:m.id,baseUrl:''})),defaultModel:'',credentialRef:'',proxy:null,enabled:true,builtinModels:[],disabledModels:[]});
const blankModel=(p:ProviderInfo):ModelConfig=>({id:crypto.randomUUID(),name:'',providerId:p.id,compatibility:p.compatibility,apiMode:p.endpoints[0]?.apiMode??'chatCompletions',baseUrl:p.endpoints[0]?.baseUrl??'',model:p.defaultModel,credentialRef:p.credentialRef,proxy:p.proxy,maxOutputTokens:null,efforts:[],effort:null,enabled:true,builtin:false});
const snapshot=(p:ProviderInfo)=>JSON.stringify(p);
const snapshotModel=(m:ModelConfig)=>JSON.stringify(m);
export function ModelPanel({models,onClose,onSaved}:{models:ModelConfig[];onClose:()=>void;onSaved:()=>Promise<void>}) {
 const [providers,setProviders]=useState<ProviderInfo[]>([]),[provider,setProvider]=useState<ProviderInfo|null>(null);
 const [draft,setDraft]=useState<ModelConfig|null>(null),[viewBuiltin,setViewBuiltin]=useState<string|null>(null);
 const [secret,setSecret]=useState(''),[error,setError]=useState(''),[status,setStatus]=useState(''),[busy,setBusy]=useState(false);
 const [fetched,setFetched]=useState<string[]>([]);
 const [savedProvider,setSavedProvider]=useState(''),[savedModel,setSavedModel]=useState('');
 const originals=useRef<ProviderInfo[]>([]),revision=useRef(0),alive=useRef(true),locked=useRef(false);
 useEffect(()=>{alive.current=true;void command<ProviderInfo[]>('provider_list').then(list=>{if(!alive.current)return;originals.current=list;setProviders(list);if(list[0])choose(list[0])}).catch(e=>{if(alive.current)setError(errorText(e))});return()=>{alive.current=false;revision.current++}},[]);
 function changed(){revision.current++;setStatus('');setError('')}
 function choose(p:ProviderInfo){changed();const fresh=structuredClone(p);setProvider(fresh);setDraft(null);setViewBuiltin(null);setFetched([]);setSecret('');setSavedProvider(snapshot(originals.current.find(x=>x.id===p.id)??p));setSavedModel('')}
 function editProvider(next:ProviderInfo){changed();setProvider(next)}
 function editModel(next:ModelConfig){changed();setDraft(next)}
 function startModel(){if(!provider)return;changed();setViewBuiltin(null);setFetched([]);const fresh=blankModel(provider);setDraft(fresh);setSavedModel('')}
 function startEditModel(m:ModelConfig){changed();setViewBuiltin(null);setFetched([]);setDraft(structuredClone(m));setSavedModel(snapshotModel(m))}
 function viewBuiltinModel(id:string){changed();setDraft(null);setFetched([]);setViewBuiltin(id)}
 async function action(fn:(token:number)=>Promise<void>){if(locked.current)return;locked.current=true;const token=revision.current;setBusy(true);setStatus('');setError('');try{await fn(token)}catch(e){if(alive.current&&token===revision.current)setError(errorText(e))}finally{locked.current=false;if(alive.current)setBusy(false)}}
 const current=(token:number)=>alive.current&&token===revision.current;
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
 })}
 async function saveModel(e:FormEvent){e.preventDefault();if(!draft||!provider)return;const m=draft;await action(async token=>{
  const next={...m,providerId:provider.id,compatibility:provider.compatibility,baseUrl:provider.endpoints.find(x=>x.apiMode===m.apiMode)?.baseUrl??m.baseUrl,credentialRef:provider.credentialRef,proxy:provider.proxy};
  await command('model_save',{config:next});if(!current(token))return;
  setDraft(structuredClone(next));setSavedModel(snapshotModel(next));await refreshLists();if(current(token))setStatus('已保存模型。');
 })}
 async function toggleProvider(p:ProviderInfo,enabled:boolean){await action(async token=>{const stored=await command<ProviderInfo>('provider_save',{config:{...p,enabled},secret:undefined});if(!current(token))return;originals.current=[...originals.current.filter(x=>x.id!==stored.id),stored];if(provider?.id===stored.id){setProvider(structuredClone(stored));setSavedProvider(snapshot(stored))}await refreshLists()})}
 async function toggleBuiltinModel(p:ProviderInfo,modelId:string,enabled:boolean){await action(async token=>{const disabled=p.disabledModels.filter(id=>id!==modelId);if(!enabled)disabled.push(modelId);const stored=await command<ProviderInfo>('provider_save',{config:{...p,disabledModels:disabled},secret:undefined});if(!current(token))return;originals.current=[...originals.current.filter(x=>x.id!==stored.id),stored];if(provider?.id===stored.id)setProvider(structuredClone(stored));await refreshLists()})}
 async function toggleModel(m:ModelConfig,enabled:boolean){await action(async token=>{await command('model_save',{config:{...m,enabled}});if(!current(token))return;await refreshLists()})}
 async function copyProvider(){if(!provider)return;await action(async token=>{const p=await command<ProviderInfo>('provider_copy',{id:provider.id});if(!current(token))return;originals.current=[...originals.current,p];setProviders(list=>[...list,p]);choose(p)})}
 async function removeProvider(){if(!provider)return;const id=provider.id;await action(async token=>{if(originals.current.some(p=>p.id===id))await command('provider_remove',{id});if(!current(token))return;originals.current=originals.current.filter(p=>p.id!==id);const list=providers.filter(p=>p.id!==id);setProviders(list);if(list[0])choose(list[0]);else{setProvider(null);setDraft(null)}})}
 async function removeModel(){if(!draft)return;const id=draft.id;await action(async token=>{await command('model_remove',{modelId:id});if(!current(token))return;setDraft(null);setSavedModel('');await refreshLists()})}
 async function fetchAvailable(){if(!provider||!draft)return;await action(async token=>{const ids=await command<string[]>('model_fetch_available',{providerId:provider.id,apiMode:draft.apiMode});if(current(token))setFetched(ids)})}
 const providerDirty=!!provider&&snapshot(provider)!==savedProvider;
 const modelClean=!!draft&&snapshotModel(draft)===savedModel;
 const savedDraft=!!draft&&models.some(m=>m.id===draft.id);
 const builtinView=provider?.builtinModels.find(m=>m.id===viewBuiltin);
 return <Modal wide title="模型设置" onClose={onClose}><p className="muted">先选择供应商并填写密钥，再在该供应商下添加或启用模型；发送框按“供应商 → 模型 → 档位”选择。密钥仅存入系统凭据管理器。</p>
 {error&&<p role="alert" className="inline-error">{error}</p>}{status&&<p role="status" className="success">{status}</p>}
 <div className="provider-layout"><section aria-label="供应商目录"><h3>供应商</h3><div className="provider-list">{providers.map(p=><div key={p.id} className={`provider-row${p.id===provider?.id?' selected':''}`}><button className="provider-name" onClick={()=>choose(p)}>{p.name}{p.builtin&&<em className="builtin-badge">内置</em>}{!p.enabled&&<em className="off-badge">已停用</em>}</button><label className="switch" title={p.enabled?'停用供应商':'启用供应商'}><input type="checkbox" aria-label={`启用供应商 ${p.name}`} checked={p.enabled} disabled={busy} onChange={e=>toggleProvider(p,e.target.checked)}/></label></div>)}</div><button disabled={busy||!desktop()} onClick={()=>{const p=blankProvider();setProviders(list=>[...list,p]);choose(p)}}>添加自定义供应商</button>
 {provider&&<><h3>{provider.name} 的模型</h3><div className="provider-list">{provider.builtinModels.map(m=>{const enabled=provider.enabled&&!provider.disabledModels.includes(m.id);return <div key={m.id} className={`provider-row${viewBuiltin===m.id?' selected':''}`}><button className="provider-name" onClick={()=>viewBuiltinModel(m.id)}>{m.name}<em className="builtin-badge">内置</em>{!enabled&&<em className="off-badge">已停用</em>}</button><label className="switch" title={enabled?'停用内置模型':'启用内置模型'}><input type="checkbox" aria-label={`启用模型 ${m.name}`} checked={enabled} disabled={busy||!provider.enabled} onChange={e=>toggleBuiltinModel(provider,m.id,e.target.checked)}/></label></div>})}{models.filter(m=>m.providerId===provider.id).map(m=><div key={m.id} className={`provider-row${draft?.id===m.id?' selected':''}`}><button className="provider-name" onClick={()=>startEditModel(m)}>{m.name} · {m.model}{!m.enabled&&<em className="off-badge">已停用</em>}</button><label className="switch" title={m.enabled?'停用模型':'启用模型'}><input type="checkbox" aria-label={`启用模型 ${m.name}`} checked={m.enabled} disabled={busy} onChange={e=>toggleModel(m,e.target.checked)}/></label></div>)}{!provider.builtinModels.length&&!models.some(m=>m.providerId===provider.id)&&<p className="muted">该供应商还没有模型，点击下方“添加模型”。</p>}</div><button disabled={busy||!provider.enabled||!desktop()} onClick={startModel}>添加模型</button></>}</section>
 {provider?<div className="provider-forms"><form onSubmit={persistProvider} className="settings-form"><h3>供应商：{provider.name}</h3><label>供应商名称<input required readOnly={provider.builtin} value={provider.name} onChange={e=>editProvider({...provider,name:e.target.value})}/></label>
 <label>API 密钥<input type="password" autoComplete="new-password" value={secret} placeholder={provider.credentialRef?'留空保留已保存密钥':'输入此供应商的密钥'} onChange={e=>{changed();setSecret(e.target.value)}}/></label>
 <label>HTTP 代理（可选）<input value={provider.proxy??''} placeholder="http://127.0.0.1:7890" onChange={e=>editProvider({...provider,proxy:e.target.value||null})}/></label>
 {provider.builtin&&<small>内置供应商名称与端点固定，可启用/停用或复制后修改。</small>}
 <div className="provider-actions"><button type="button" disabled={busy||!originals.current.some(p=>p.id===provider.id)} onClick={copyProvider}>复制供应商</button>{!provider.builtin&&<button type="button" className="danger" disabled={busy||models.some(m=>m.providerId===provider.id)} onClick={removeProvider}>删除供应商</button>}</div>
 {!provider.builtin&&models.some(m=>m.providerId===provider.id)&&<small>删除供应商前，请先逐一删除关联的模型。</small>}
 {!provider.builtin&&<label>默认模型<input value={provider.defaultModel} onChange={e=>editProvider({...provider,defaultModel:e.target.value})}/></label>}
 <div className="actions"><button className="primary" disabled={busy||!desktop()||(!providerDirty&&!secret)}>{busy?'处理中…':'保存供应商'}</button></div>
 </form>
 {draft?<form onSubmit={saveModel} className="settings-form"><h3>{savedDraft?'编辑模型':'添加模型'}</h3><label>显示名称<input required value={draft.name} onChange={e=>editModel({...draft,name:e.target.value})}/></label>
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
 <div className="actions model-actions"><button type="button" disabled={busy||!savedDraft||!modelClean} onClick={()=>action(async token=>{await command<string>('model_test',{modelId:draft.id});if(current(token))setStatus('模型连接成功。')})}>测试已保存配置</button><button type="button" className="danger" disabled={busy||!savedDraft} onClick={removeModel}>删除模型</button><button className="primary" disabled={busy||!desktop()}>{busy?'处理中…':'保存模型'}</button></div>
 </form>:builtinView?<section aria-label="内置模型信息" className="settings-form"><h3>内置模型：{builtinView.name}</h3><p>模型标识 <code>{builtinView.id}</code>（内置目录成员，不可修改或删除，可启用/停用）。</p><p>支持档位：{builtinView.efforts.length?builtinView.efforts.join(' / '):'无'}；协议固定为 Anthropic 端点。</p><p className="muted">启用后即可在发送框的模型列表中选择。</p></section>:<p className="muted">选择左侧模型进行编辑，或点击“添加模型”。</p>}</div>:<p className="muted">{busy?'处理中…':'正在加载供应商…'}</p>}</div></Modal>
}
