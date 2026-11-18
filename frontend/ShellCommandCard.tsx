import {useLayoutEffect,useRef,useState,type ReactNode} from 'react';
import {ChevronRight,Terminal} from 'lucide-react';
import {parseShellResult,type ShellTimelineNode} from './shellEvents';

type ShellNode=Extract<ShellTimelineNode,{kind:'shell'}>;

export function ShellCommandCard({node,artifact}:{node:ShellNode;artifact?:ReactNode}) {
 const args=node.call?.data.arguments;
 const command=args&&typeof args==='object'&&typeof (args as Record<string,unknown>).command==='string'?(args as Record<string,string>).command:'';
 const status=node.result?.data.status;
 const label=!node.result?'等待结果':status==='denied'?'已拒绝':status==='cancelled'?'已取消':node.result.data.isError===true||status==='error'?'执行失败':'已完成';
 const parsed=node.result?parseShellResult(node.result.data.content):null;
 const [expanded,setExpanded]=useState(false),[overflow,setOverflow]=useState(command.split('\n').length>3);
 const commandRef=useRef<HTMLDivElement>(null);
 useLayoutEffect(()=>{
  const el=commandRef.current;
  if(!el||expanded)return;
  const measure=()=>setOverflow(command.split('\n').length>3||el.scrollHeight>el.clientHeight+1);
  measure();
  if(typeof ResizeObserver==='undefined')return;
  const observer=new ResizeObserver(measure);observer.observe(el);return()=>observer.disconnect();
 },[command,expanded]);
 return <details className="tool-card shell-card"><summary><Terminal size={15} aria-hidden="true"/><strong>执行命令</strong><span className="shell-summary-command">{command||'命令不可用'}</span><span className={`shell-status${label==='执行失败'||label==='已拒绝'?' is-error':''}`}>{label}</span><ChevronRight size={14} className="chev" aria-hidden="true"/></summary>
  <div className="shell-detail">
   <div className="shell-command"><span aria-hidden="true">$</span><div className="shell-command-body"><div ref={commandRef} className={`shell-command-text${expanded?' expanded':''}`}>{command||'命令不可用'}</div>{(overflow||expanded)&&<button type="button" className="shell-command-toggle" aria-expanded={expanded} onClick={()=>setExpanded(value=>!value)}>{expanded?'收起命令':'展开完整命令'}</button>}</div></div>
   {parsed?.kind==='output'?<div className="shell-output">{parsed.stdout&&<pre>{parsed.stdout}</pre>}{parsed.stderr&&<pre className="shell-stderr">{parsed.stderr}</pre>}{!parsed.stdout&&!parsed.stderr&&<span>无输出</span>}</div>:parsed?.kind==='raw'?<div className="shell-output"><pre>{parsed.text||'无输出'}</pre></div>:<div className="shell-output">等待结果</div>}
   {parsed?.kind==='output'&&<div className="shell-exit">退出码 {parsed.exitCode}</div>}
   {artifact}
  </div>
 </details>;
}
