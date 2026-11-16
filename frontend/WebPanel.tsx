import {useEffect,useRef,useState} from 'react';
import {ArrowLeft,ArrowRight,RotateCw} from 'lucide-react';
import {command,errorText} from './api';
import {isTauri} from '@tauri-apps/api/core';
import {listen} from '@tauri-apps/api/event';
import {useSidePanel} from './sidePanelStore';
let queue:Promise<void>=Promise.resolve();
function enqueue(action:()=>Promise<unknown>,onError?:(error:unknown)=>void){queue=queue.catch(()=>{}).then(async()=>{try{await action()}catch(error){onError?.(error)}})}
export function WebPanel({url}:{url:string|null}) {
 const area=useRef<HTMLDivElement>(null),[error,setError]=useState('');
 const [liveUrl,setLiveUrl]=useState<string|null>(url),[canBack,setCanBack]=useState(false),[canForward,setCanForward]=useState(false);
 // 已应用的地址：load-status 回显后跳过重复打开，避免 open→事件→open 循环。
 const applied=useRef<string|null>(null);
 async function history(action:string){try{await command('web_panel_history',{action})}catch(e){setError(errorText(e))}}
 // 打开/换页：web_panel_open 幂等（webview 已存在则导航，不重建）。
 useEffect(()=>{
  if(!url||applied.current===url)return;
  applied.current=url;
  setError('');
  setLiveUrl(url);
  enqueue(async()=>{await command('web_panel_open',{url})},e=>{setError(errorText(e))});
 },[url]);
 // 事件订阅与 bounds 同步；卸载仅隐藏 webview（面板 webview 持久化，供模型继续操作）。
 useEffect(()=>{
  let active=true,unlisteners:Array<()=>void>=[];
  const listeners=isTauri()?Promise.all([
   listen<[boolean,string|null]>('web-panel-load-status',event=>{if(!active)return;const[ok,current]=event.payload;setLiveUrl(current);applied.current=current;useSidePanel.getState().updateWebUrl(current);setError(ok?'':'网页加载失败，请检查网络或在系统浏览器中打开。')}),
   listen('web-panel-close-request',()=>{if(active)useSidePanel.getState().close()}),
   listen<[boolean,boolean]>('web-panel-history',event=>{if(!active)return;setCanBack(event.payload[0]);setCanForward(event.payload[1])})
  ]).then(stops=>{if(active)unlisteners=stops;else stops.forEach(stop=>stop())}):Promise.resolve();
  let lastBounds='';
  const sync=()=>{
   if(!area.current)return;
   const rect=area.current.getBoundingClientRect();
   const visible=!document.querySelector('.overlay, .link-context-menu')&&rect.width>0&&rect.height>0;
   const next=JSON.stringify([rect.left,rect.top,rect.width,rect.height,visible]);if(next===lastBounds)return;lastBounds=next;
   enqueue(()=>command('web_panel_bounds',{x:rect.left,y:rect.top,width:rect.width,height:rect.height,visible}),e=>{if(active)setError(errorText(e))});
  };
  const resize=typeof ResizeObserver!=='undefined'?new ResizeObserver(sync):null;if(area.current)resize?.observe(area.current);
  const observer=new MutationObserver(sync);observer.observe(document.body,{childList:true,subtree:true});
  window.addEventListener('resize',sync);sync();
  return()=>{active=false;unlisteners.forEach(stop=>stop());resize?.disconnect();observer.disconnect();window.removeEventListener('resize',sync);enqueue(()=>command('web_panel_bounds',{x:0,y:0,width:0,height:0,visible:false}))};
 },[]);
 async function external(){const target=liveUrl??url;if(!target)return;try{await command('open_external_link',{url:target})}catch(e){setError(errorText(e))}}
 return <div className="web-panel"><div className="web-panel-address">
  <button type="button" aria-label="后退" disabled={!canBack} onClick={()=>void history('back')}><ArrowLeft size={15}/></button>
  <button type="button" aria-label="前进" disabled={!canForward} onClick={()=>void history('forward')}><ArrowRight size={15}/></button>
  <button type="button" aria-label="刷新" onClick={()=>void history('reload')}><RotateCw size={15}/></button>
  <span title={liveUrl??url??''}>{liveUrl??url??'请选择网页链接'}</span>
  {(liveUrl??url)&&<button type="button" onClick={()=>void external()}>浏览器打开</button>}
 </div>{error&&<div role="alert" className="inline-error">网页无法打开：{error}</div>}<div className="web-panel-content" ref={area}/></div>;
}
