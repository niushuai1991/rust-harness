import {useEffect,useState,type ReactNode,type MouseEvent} from 'react';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';
import {command,errorText} from './api';
import {useSidePanel} from './sidePanelStore';

export function httpUrl(value:string):boolean {
 try {const url=new URL(value);return url.protocol==='http:'||url.protocol==='https:'} catch {return false}
}
function SessionLink({href,children}:{href?:string;children?:ReactNode}) {
 const [menu,setMenu]=useState<{x:number;y:number}|null>(null),[error,setError]=useState('');
 const valid=!!href&&httpUrl(href);
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
 return <><a href={href} onClick={event=>{if(valid){event.preventDefault();useSidePanel.getState().openWeb(href!)}}} onContextMenu={context}>{children}</a>
  {menu&&<div className="session-context-menu link-context-menu" role="menu" aria-label="链接操作" style={{left:menu.x,top:menu.y}} onMouseDown={event=>event.stopPropagation()}><button type="button" role="menuitem" onClick={()=>void perform('copy')}>复制链接</button><button type="button" role="menuitem" onClick={()=>void perform('open')}>在默认浏览器中打开</button></div>}
  {error&&<span role="alert" className="inline-error">{error}</span>}</>;
}
export function SessionMarkdown({content}:{content:string}) {return <ReactMarkdown remarkPlugins={[remarkGfm]} components={{a:({href,children})=><SessionLink href={href}>{children}</SessionLink>}}>{content}</ReactMarkdown>}
export function LinkedText({content}:{content:string}) {
 const parts=content.split(/(https?:\/\/[^\s<>"'，。！？、；：）】]+)/gi);
 return <>{parts.map((part,index)=>{
  if(!/^https?:\/\//i.test(part))return part;
  let url=part.replace(/[.,;:!?]+$/,'');
  while(url.endsWith(')')&&(url.match(/\)/g)?.length??0)>(url.match(/\(/g)?.length??0))url=url.slice(0,-1);
  return httpUrl(url)?<span key={index}><SessionLink href={url}>{url}</SessionLink>{part.slice(url.length)}</span>:part;
 })}</>;
}
