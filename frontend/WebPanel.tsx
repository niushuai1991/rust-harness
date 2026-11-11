import {useEffect,useRef,useState} from 'react';
import {command,errorText} from './api';
import {isTauri} from '@tauri-apps/api/core';
import {listen} from '@tauri-apps/api/event';
import {useSidePanel} from './sidePanelStore';
let queue:Promise<void>=Promise.resolve();
let nextGeneration=0;
function enqueue(action:()=>Promise<unknown>,onError?:(error:unknown)=>void){queue=queue.catch(()=>{}).then(async()=>{try{await action()}catch(error){onError?.(error)}})}
export function WebPanel({url}:{url:string|null}) {
 const area=useRef<HTMLDivElement>(null),[error,setError]=useState('');
 useEffect(()=>{
  if(!url)return;
  let active=true;
  setError('');
  const generation=String(++nextGeneration);
  let unlisteners:Array<()=>void>=[];
  const listeners=isTauri()?Promise.all([
   listen<[string,boolean]>('web-panel-load-status',event=>{if(active&&event.payload[0]===generation)setError(event.payload[1]?'':'网页加载失败，请检查网络或在系统浏览器中打开。')}),
   listen<string>('web-panel-close-request',event=>{if(active&&event.payload===generation)useSidePanel.getState().close()})
  ]).then(stops=>{if(active)unlisteners=stops;else stops.forEach(stop=>stop())}):Promise.resolve();
  enqueue(async()=>{await listeners;if(active)await command('web_panel_open',{url,generation})},e=>{if(active)setError(errorText(e))});
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
  return()=>{active=false;unlisteners.forEach(stop=>stop());resize?.disconnect();observer.disconnect();window.removeEventListener('resize',sync);enqueue(()=>command('web_panel_close'))};
 },[url]);
 async function external(){if(!url)return;try{await command('open_external_link',{url})}catch(e){setError(errorText(e))}}
 return <div className="web-panel"><div className="web-panel-address"><span title={url??''}>{url??'请选择网页链接'}</span>{url&&<button type="button" onClick={()=>void external()}>浏览器打开</button>}</div>{error&&<div role="alert" className="inline-error">网页无法打开：{error}</div>}<div className="web-panel-content" ref={area}/></div>;
}
