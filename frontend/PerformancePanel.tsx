import {useEffect,useState} from 'react';
import {command,errorText} from './api';

type PerfSettings={toolConcurrency:boolean;toolConcurrencyLimit:number;autoCompaction:boolean;httpConnectionReuse:boolean};

const DEFAULTS:PerfSettings={toolConcurrency:true,toolConcurrencyLimit:8,autoCompaction:true,httpConnectionReuse:true};

function Toggle({label,description,checked,onChange}:{label:string;description:string;checked:boolean;onChange:(value:boolean)=>void}){
 return <article className="settings-example-item">
  <span className="settings-example-copy"><strong>{label}</strong><small>{description}</small></span>
  <button type="button" role="switch" aria-checked={checked} aria-label={label} className={`perf-toggle${checked?' on':''}`} onClick={()=>onChange(!checked)}><span className="perf-toggle-knob"/></button>
 </article>;
}

export function PerformancePanel(){
 const [settings,setSettings]=useState<PerfSettings|null>(null);
 const [limitText,setLimitText]=useState('8');
 const [error,setError]=useState<string|null>(null);
 useEffect(()=>{
  command<PerfSettings>('perf_settings_get')
   .then(value=>{setSettings(value);setLimitText(String(value.toolConcurrencyLimit))})
   .catch(e=>setError(errorText(e)));
 },[]);
 const update=(patch:Partial<PerfSettings>)=>{
  if(!settings)return;
  const next={...settings,...patch};
  setSettings(next);
  setError(null);
  command('perf_settings_set',next).catch(e=>{setError(errorText(e));setSettings(settings)});
 };
 const applyLimit=()=>{
  if(!settings)return;
  const parsed=Number.parseInt(limitText,10);
  if(!Number.isFinite(parsed)||parsed<1||parsed>16){
   setError('工具并发上限必须在 1–16 之间');
   setLimitText(String(settings.toolConcurrencyLimit));
   return;
  }
  update({toolConcurrencyLimit:parsed});
 };
 if(!settings)return <div className="settings-example-list" aria-busy="true">{error?<p className="muted">{error}</p>:<p className="muted">正在加载性能设置…</p>}</div>;
 return <div className="settings-example-list settings-performance-list">
  <Toggle label="工具并行调度" description="同一响应中的只读工具（读取文件、搜索、列表等）并发执行；关闭后保持逐个串行。" checked={settings.toolConcurrency} onChange={value=>update({toolConcurrency:value})}/>
  <article className="settings-example-item">
   <span className="settings-example-copy"><strong>并行数量上限</strong><small>并发段内同时执行的工具数上限（1–16，默认 8）。</small></span>
   <input type="number" min={1} max={16} aria-label="工具并行数量上限" value={limitText} disabled={!settings.toolConcurrency}
    onChange={e=>setLimitText(e.target.value)} onBlur={applyLimit}
    onKeyDown={e=>{if(e.key==='Enter'){e.preventDefault();applyLimit()}}}/>
  </article>
  <Toggle label="上下文自动压缩" description="输入接近模型窗口阈值时，在下一次请求前自动压缩会话投影；原始历史始终保留。" checked={settings.autoCompaction} onChange={value=>update({autoCompaction:value})}/>
  <Toggle label="HTTP 连接复用" description="对同一供应商配置复用长连接与连接池；关闭后每次模型调用新建连接。" checked={settings.httpConnectionReuse} onChange={value=>update({httpConnectionReuse:value})}/>
  {error&&<p className="muted" role="alert">{error}</p>}
 </div>;
}
