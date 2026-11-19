import {useEffect,useRef,useState} from 'react';
import {createPortal} from 'react-dom';
import {fitPreview} from './turnNavigatorText';
import type {TurnNavigatorEntry} from './types';

type Props={entries:TurnNavigatorEntry[];activeId:string|null;onJump:(id:string)=>void;onLoadOlder:()=>void;hasOlder:boolean;loading:boolean};
export function TurnNavigator({entries,activeId,onJump,onLoadOlder,hasOlder,loading}:Props){
 const list=useRef<HTMLDivElement>(null),[labels,setLabels]=useState<Record<string,string>>({}),[tip,setTip]=useState<{entry:TurnNavigatorEntry;left:number;top:number}|null>(null);
 useEffect(()=>{const node=list.current;if(!node||typeof ResizeObserver==='undefined')return;const canvas=document.createElement('canvas'),context=canvas.getContext('2d');if(!context)return;const fit=()=>{const button=node.querySelector('button.turn-nav-item');if(!button)return;const style=getComputedStyle(button);context.font=style.font;const width=Math.max(0,button.clientWidth-parseFloat(style.paddingLeft)-parseFloat(style.paddingRight));if(!width)return;setLabels(Object.fromEntries(entries.map(e=>[e.eventId,fitPreview(e.preview,width,t=>context.measureText(t).width)])))};const observer=new ResizeObserver(fit);observer.observe(node);fit();return()=>observer.disconnect()},[entries]);
 useEffect(()=>{if(!activeId)return;Array.from(list.current?.querySelectorAll<HTMLElement>('[data-nav-id]')??[]).find(el=>el.dataset.navId===activeId)?.scrollIntoView?.({block:'nearest'})},[activeId]);
 if(!entries.length)return null;
 const show=(element:HTMLElement,entry:TurnNavigatorEntry)=>{const rect=element.getBoundingClientRect();setTip({entry,left:Math.max(8,Math.min(rect.right+8,window.innerWidth-332)),top:Math.max(8,Math.min(rect.top,window.innerHeight-160))})};
 return <nav className="turn-nav" aria-label="会话回合导航"><div ref={list} className="turn-nav-list" onScroll={e=>{setTip(null);if(e.currentTarget.scrollTop<=8&&hasOlder&&!loading)onLoadOlder()}}>
  {hasOlder&&<button className="turn-nav-more" onClick={onLoadOlder} disabled={loading}>{loading?'…':'↑'}</button>}
  {entries.map(entry=><button key={entry.eventId} type="button" data-nav-id={entry.eventId} className="turn-nav-item" aria-label={`跳转到：${entry.preview}`} aria-current={entry.eventId===activeId?'location':undefined} onMouseEnter={e=>show(e.currentTarget,entry)} onMouseLeave={()=>setTip(null)} onFocus={e=>show(e.currentTarget,entry)} onBlur={()=>setTip(null)} onClick={()=>{setTip(null);onJump(entry.eventId)}}><span>{labels[entry.eventId]??entry.preview}</span></button>)}
 </div>{tip&&createPortal(<div role="tooltip" className="turn-nav-tooltip" style={{left:tip.left,top:tip.top}}>{tip.entry.preview}{tip.entry.previewTruncated?'…':''}</div>,document.body)}</nav>;
}
