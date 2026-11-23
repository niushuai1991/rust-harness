import {useEffect,useMemo,useState,type ReactNode,type MouseEvent} from 'react';
import {Check,Copy,X} from 'lucide-react';
import ReactMarkdown, {defaultUrlTransform,type Components} from 'react-markdown';
import remarkGfm from 'remark-gfm';
import {command,errorText} from './api';
import {useSidePanel} from './sidePanelStore';

export function httpUrl(value:string):boolean {
 try {const url=new URL(value);return url.protocol==='http:'||url.protocol==='https:'} catch {return false}
}
function localPdfPath(value:string):string|null {
 if(httpUrl(value))return null;
 try {
  if(/^file:\/\//i.test(value)) {
   const url=new URL(value);
   if(url.hostname&&url.hostname!=='localhost')return null;
   value=decodeURIComponent(url.pathname).replace(/^\/([a-z]:\/)/i,'$1');
  } else value=decodeURIComponent(value);
 } catch {return null}
 if(!/\.pdf$/i.test(value)||(/^[a-z][a-z0-9+.-]*:/i.test(value)&&!/^[a-z]:[\\/]/i.test(value)))return null;
 return value;
}
function SessionLink({href,children}:{href?:string;children?:ReactNode}) {
 const [menu,setMenu]=useState<{x:number;y:number}|null>(null),[error,setError]=useState('');
 const valid=!!href&&httpUrl(href),pdf=href?localPdfPath(href):null;
 useEffect(()=>{
  if(!menu)return;
  const dismiss=()=>setMenu(null);
  const key=(event:KeyboardEvent)=>{if(event.key==='Escape'){event.stopPropagation();setMenu(null)}};
  document.addEventListener('mousedown',dismiss);document.addEventListener('keydown',key,true);
  return()=>{document.removeEventListener('mousedown',dismiss);document.removeEventListener('keydown',key,true)};
 },[menu]);
 const context=(event:MouseEvent)=>{event.preventDefault();if(valid)setMenu({x:Math.max(4,Math.min(event.clientX,window.innerWidth-190)),y:Math.max(4,Math.min(event.clientY,window.innerHeight-90))})};
 async function perform(action:'copy'|'open') {
  setMenu(null);setError('');
  try {if(action==='copy')await navigator.clipboard.writeText(href!);else await command('open_external_link',{url:href})}
  catch(e){setError(errorText(e))}
 }
 return <><a href={href} onClick={event=>{if(pdf){event.preventDefault();useSidePanel.getState().openPdf(pdf)}else if(valid){event.preventDefault();useSidePanel.getState().openWeb(href!)}}} onContextMenu={context}>{children}</a>
  {menu&&<div className="session-context-menu link-context-menu" role="menu" aria-label="链接操作" style={{left:menu.x,top:menu.y}} onMouseDown={event=>event.stopPropagation()}><button type="button" role="menuitem" onClick={()=>void perform('copy')}>复制链接</button><button type="button" role="menuitem" onClick={()=>void perform('open')}>在默认浏览器中打开</button></div>}
  {error&&<span role="alert" className="inline-error">{error}</span>}</>;
}
function CodeBlock({raw,children}:{raw:string;children?:ReactNode}) {
 const [state,setState]=useState<'idle'|'success'|'error'>('idle');
 useEffect(()=>{if(state==='idle')return;const timer=window.setTimeout(()=>setState('idle'),2000);return()=>window.clearTimeout(timer)},[state]);
 async function copy(){try{await navigator.clipboard.writeText(raw);setState('success')}catch{setState('error')}}
 const language=raw.match(/^```([^\r\n]*)/)?.[1].trim().split(/\s+/)[0]||'text';
 return <div className="markdown-code-block"><div className="markdown-code-header"><span>{language}</span><button type="button" className={`markdown-code-copy ${state}`} aria-label="复制代码块" title={state==='success'?'已复制':state==='error'?'复制失败':'复制代码块'} onClick={()=>void copy()}>{state==='success'?<Check size={15}/>:state==='error'?<X size={15}/>:<Copy size={15}/>}</button></div><pre>{children}</pre><span className="markdown-code-status" role="status">{state==='success'?'已复制':state==='error'?'复制失败':''}</span></div>;
}
export function SessionMarkdown({content,sessionLinks=true}:{content:string;sessionLinks?:boolean}) {
 const components=useMemo<Components>(()=>({
  ...(sessionLinks?{a:({href,children})=><SessionLink href={href}>{children}</SessionLink>}:{}),
  pre:({node,children})=>{
   const start=node?.position?.start.offset,end=node?.position?.end.offset;
   const raw=start!==undefined&&end!==undefined?content.slice(start,end):'';
   if(!/^```(?!`)[^`\r\n]*(?:\r\n|\r|\n)/.test(raw)||!/(?:^|\r|\n)[ \t]*(?:> ?)*```[ \t]*$/.test(raw))return <pre>{children}</pre>;
   return <CodeBlock raw={raw}>{children}</CodeBlock>;
  }
 }),[content,sessionLinks]);
 return <ReactMarkdown urlTransform={(url)=>sessionLinks&&localPdfPath(url)?url:defaultUrlTransform(url)} remarkPlugins={[remarkGfm]} components={components}>{content}</ReactMarkdown>;
}
export function LinkedText({content}:{content:string}) {
 const parts=content.split(/(https?:\/\/[^\s<>"'，。！？、；：）】]+|[a-z]:[\\/][^\r\n<>"'，。！？、；：）】]*?\.pdf(?=$|[\s.,;:!?)，。！？、；：）】]))/gi);
 return <>{parts.map((part,index)=>{
  if(!/^https?:\/\//i.test(part))return /^[a-z]:[\\/]/i.test(part)&&localPdfPath(part)?<SessionLink key={index} href={part}>{part}</SessionLink>:part;
  let url=part.replace(/[.,;:!?]+$/,'');
  while(url.endsWith(')')&&(url.match(/\)/g)?.length??0)>(url.match(/\(/g)?.length??0))url=url.slice(0,-1);
  return httpUrl(url)?<span key={index}><SessionLink href={url}>{url}</SessionLink>{part.slice(url.length)}</span>:part;
 })}</>;
}
