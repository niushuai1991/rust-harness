import {useEffect,useRef,useState,type FormEvent} from 'react';
import {command,desktop,errorText} from './api';
import {Modal} from './App';
import type {ApiMode,ModelConfig,ProviderInfo} from './types';
const modes:{id:ApiMode;name:string}[]=[{id:'chatCompletions',name:'Chat Completions'},{id:'responses',name:'Responses'},{id:'anthropic',name:'Anthropic'}];
const blankModel=(p:ProviderInfo):ModelConfig=>({id:crypto.randomUUID(),name:p.name,providerId:p.id,compatibility:p.compatibility,apiMode:p.endpoints[0]?.apiMode??'chatCompletions',baseUrl:p.endpoints[0]?.baseUrl??'',model:p.defaultModel,credentialRef:p.credentialRef,proxy:p.proxy,maxOutputTokens:null});
const snapshot=(p:ProviderInfo,m:ModelConfig)=>JSON.stringify([p,m]);
export function ModelPanel({models,onClose,onSaved}:{models:ModelConfig[];onClose:()=>void;onSaved:()=>Promise<void>}) {
 const [providers,setProviders]=useState<ProviderInfo[]>([]),[provider,setProvider]=useState<ProviderInfo|null>(null),[config,setConfig]=useState<ModelConfig|null>(null);
 const [secret,setSecret]=useState(''),[error,setError]=useState(''),[status,setStatus]=useState(''),[busy,setBusy]=useState(false),[savedSnapshot,setSavedSnapshot]=useState('');
 const originals=useRef<ProviderInfo[]>([]),revision=useRef(0),alive=useRef(true),locked=useRef(false);
 useEffect(()=>{alive.current=true;void command<ProviderInfo[]>('provider_list').then(list=>{if(!alive.current)return;originals.current=list;setProviders(list);if(list[0]){setProvider(list[0]);setConfig(blankModel(list[0]))}}).catch(e=>{if(alive.current)setError(errorText(e))});return()=>{alive.current=false;revision.current++}},[]);
 function changed(){revision.current++;setStatus('');setError('')}
 function choose(p:ProviderInfo,m?:ModelConfig){changed();setProvider(p);const next=m?{...m,baseUrl:p.endpoints.find(e=>e.apiMode===m.apiMode)?.baseUrl??'',credentialRef:p.credentialRef,proxy:p.proxy,compatibility:p.compatibility}:blankModel(p);setConfig(next);setSecret('');setSavedSnapshot(m?snapshot(originals.current.find(x=>x.id===p.id)??p,m):'')}
 function editProvider(next:ProviderInfo){changed();setProvider(next);setProviders(list=>list.map(p=>p.id===next.id?next:p))}
 function editModel(next:ModelConfig){changed();setConfig(next)}
 async function action(fn:(token:number)=>Promise<void>){if(locked.current)return;locked.current=true;const token=revision.current;setBusy(true);setStatus('');setError('');try{await fn(token)}catch(e){if(alive.current&&token===revision.current)setError(errorText(e))}finally{locked.current=false;if(alive.current)setBusy(false)}}
 const current=(token:number)=>alive.current&&token===revision.current;
 async function persist(e:FormEvent){e.preventDefault();if(!provider||!config)return;const p=provider,m=config,key=secret;await action(async token=>{
  // Detect external credential/proxy changes before writing a stale supplier draft.
  const fresh=await command<ProviderInfo[]>('provider_list');const previous=originals.current.find(x=>x.id===p.id),latest=fresh.find(x=>x.id===p.id);
  if(previous&&(!latest||latest.credentialRef!==previous.credentialRef||latest.proxy!==previous.proxy))throw new Error('供应商凭据或代理已在其他位置更改，请关闭并重新打开设置后保存。');
  if(!current(token))return;
  const stored=await command<ProviderInfo>('provider_save',{config:{...p,defaultModel:p.defaultModel.trim()||m.model},secret:key||undefined});
  originals.current=[...originals.current.filter(x=>x.id!==stored.id),stored];
  const next={...m,providerId:stored.id,compatibility:stored.compatibility,baseUrl:stored.endpoints.find(x=>x.apiMode===m.apiMode)?.baseUrl??'',credentialRef:stored.credentialRef,proxy:stored.proxy};
  await command('model_save',{config:next});await onSaved();
  if(current(token)){setProvider(stored);setProviders(list=>list.map(x=>x.id===stored.id?stored:x));setConfig(next);setSecret('');setSavedSnapshot(snapshot(stored,next));setStatus('已保存；密钥由系统凭据管理器保管。')}
 })}
 async function copy(){if(!provider||!config)return;const draft=config;await action(async token=>{const p=await command<ProviderInfo>('provider_copy',{id:provider.id});originals.current=[...originals.current,p];if(current(token)){setProviders(list=>[...list,p]);choose(p);setConfig({...draft,id:crypto.randomUUID(),providerId:p.id,baseUrl:p.endpoints.find(x=>x.apiMode===draft.apiMode)?.baseUrl??'',proxy:p.proxy,credentialRef:p.credentialRef,compatibility:p.compatibility})}})}
 async function removeProvider(){if(!provider)return;const id=provider.id;await action(async token=>{if(originals.current.some(p=>p.id===id))await command('provider_remove',{id});originals.current=originals.current.filter(p=>p.id!==id);if(current(token)){const list=providers.filter(p=>p.id!==id);setProviders(list);if(list[0])choose(list[0]);else{setProvider(null);setConfig(null)}}})}
 async function removeModel(){if(!config)return;const id=config.id;await action(async token=>{await command('model_remove',{modelId:id});await onSaved();if(current(token)&&provider)choose(provider)})}
 const clean=!!provider&&!!config&&!secret&&snapshot(provider,config)===savedSnapshot&&models.some(m=>m.id===config.id);
 return <Modal wide title="模型设置" onClose={onClose}><p className="muted">先选择供应商并填写密钥，再选择协议和模型。供应商内的协议共用密钥与代理；密钥仅存入系统凭据管理器。</p>
 {error&&<p role="alert" className="inline-error">{error}</p>}{status&&<p role="status" className="success">{status}</p>}
 <div className="provider-layout"><section aria-label="供应商目录"><h3>供应商</h3><div className="provider-list">{providers.map(p=><button key={p.id} className={p.id===provider?.id?'selected':''} onClick={()=>choose(p)}>{p.name}</button>)}</div><button disabled={busy||!desktop()} onClick={()=>{const p:ProviderInfo={id:crypto.randomUUID(),name:'自定义供应商',builtin:false,compatibility:'standard',endpoints:modes.map(m=>({apiMode:m.id,baseUrl:''})),defaultModel:'',credentialRef:'',proxy:null};setProviders(list=>[...list,p]);choose(p)}}>添加自定义供应商</button>
 <h3>已保存模型</h3><div className="provider-list">{models.map(m=><button key={m.id} className={m.id===config?.id?'selected':''} onClick={()=>{const p=providers.find(x=>x.id===m.providerId);if(p)choose(p,m);else setError('该模型的供应商不存在，请重新打开设置。')}}>{m.name} · {m.model}</button>)}{!models.length&&<p className="muted">尚未保存模型配置。</p>}</div></section>
 {provider&&config?<form onSubmit={persist} className="settings-form"><label>供应商名称<input required readOnly={provider.builtin} value={provider.name} onChange={e=>editProvider({...provider,name:e.target.value})}/></label>
 <label>API 密钥<input type="password" autoComplete="new-password" value={secret} placeholder={provider.credentialRef?'留空保留已保存密钥':'输入此供应商的密钥'} onChange={e=>{changed();setSecret(e.target.value)}}/></label>
 <label>HTTP 代理（可选）<input value={provider.proxy??''} placeholder="http://127.0.0.1:7890" onChange={e=>editProvider({...provider,proxy:e.target.value||null})}/></label>
 <label>协议<select value={config.apiMode} onChange={e=>{const apiMode=e.target.value as ApiMode;if(!provider.endpoints.some(x=>x.apiMode===apiMode))editProvider({...provider,endpoints:[...provider.endpoints,{apiMode,baseUrl:''}]});editModel({...config,apiMode})}}>{modes.filter(m=>!provider.builtin||provider.endpoints.some(x=>x.apiMode===m.id)).map(m=><option key={m.id} value={m.id}>{m.name}</option>)}</select></label>
 <label>API 地址<input required type="url" readOnly={provider.builtin} value={provider.endpoints.find(x=>x.apiMode===config.apiMode)?.baseUrl??''} onChange={e=>editProvider({...provider,endpoints:provider.endpoints.map(x=>x.apiMode===config.apiMode?{...x,baseUrl:e.target.value}:x)})}/></label>
 {provider.builtin&&<small>内置供应商名称与端点固定。复制后可独立编辑。</small>}
 <div className="provider-actions"><button type="button" disabled={busy||!originals.current.some(p=>p.id===provider.id)} onClick={copy}>复制供应商</button>{!provider.builtin&&<button type="button" className="danger" disabled={busy||models.some(m=>m.providerId===provider.id)} onClick={removeProvider}>删除供应商</button>}</div>
 {!provider.builtin&&models.some(m=>m.providerId===provider.id)&&<small>删除供应商前，请先逐一删除关联的已保存模型。</small>}
 {!provider.builtin&&<label>默认模型<input value={provider.defaultModel} onChange={e=>editProvider({...provider,defaultModel:e.target.value})}/></label>}
 <label>显示名称<input required value={config.name} onChange={e=>editModel({...config,name:e.target.value})}/></label>
 <label>模型标识<input required value={config.model} onChange={e=>editModel({...config,model:e.target.value})}/></label>
 <label>最大输出 Token（可选）<input type="number" min="1" max="2147483647" step="1" value={config.maxOutputTokens??''} onChange={e=>editModel({...config,maxOutputTokens:e.target.value?Number(e.target.value):null})}/></label>
 <p className="muted">连接测试会向已保存模型发送少量请求并消耗额度。修改配置后需先保存。</p>
 <div className="actions model-actions"><button type="button" disabled={busy||!clean} onClick={()=>action(async token=>{await command<string>('model_test',{modelId:config.id});if(current(token))setStatus('模型连接成功。')})}>测试已保存配置</button><button type="button" className="danger" disabled={busy||!models.some(m=>m.id===config.id)} onClick={removeModel}>删除模型</button><button className="primary" disabled={busy||!desktop()}>{busy?'处理中…':'保存模型'}</button></div>
 </form>:<p className="muted">{busy?'处理中…':'正在加载供应商…'}</p>}</div></Modal>
}
