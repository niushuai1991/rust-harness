import {useEffect,useRef,useState,type RefObject} from 'react';
import {command,desktop,errorText} from './api';
import {waitForMessage} from './turnNavigatorText';
import {useChat} from './store';
import type {SessionEvent,TurnNavigatorEntry,TurnNavigatorPage} from './types';

type Options={sessionId:string;events:SessionEvent[];activeSessionId:RefObject<string>;autoFollow:RefObject<boolean>;conversationRef:RefObject<HTMLElement|null>;setError:(error:string)=>void;loadOlderTimeline:()=>Promise<boolean>};
export function useTurnNavigation({sessionId,events,activeSessionId,autoFollow,conversationRef,setError,loadOlderTimeline}:Options){
 const [navState,setNavState]=useState<{id:string;entries:TurnNavigatorEntry[];cursor:string|null;hasOlder:boolean}>({id:'',entries:[],cursor:null,hasOlder:false});
 const [navLoading,setNavLoading]=useState(false),[navError,setNavError]=useState(false),[navActive,setNavActive]=useState<string|null>(null),[jumping,setJumping]=useState(false);
 const navRequest=useRef(0),jumpRequest=useRef(0),navLock=useRef(false);
 useEffect(()=>{const id=sessionId,request=++navRequest.current;++jumpRequest.current;setJumping(false);setNavError(false);setNavActive(null);setNavState({id,entries:[],cursor:null,hasOlder:false});navLock.current=false;setNavLoading(false);if(!id||!desktop())return;
  void loadLatestNav(id,request);
  return()=>{++navRequest.current;++jumpRequest.current};
 },[sessionId]);
 useEffect(()=>{if(!sessionId)return;const entries=events.filter(e=>e.kind==='user.message').map(navFromEvent);if(entries.length)setNavState(old=>old.id===sessionId?{...old,entries:mergeNav(old.entries,entries)}:old)},[events,sessionId]);
 async function loadLatestNav(id:string,request:number){try{const page=await command<TurnNavigatorPage>('session_turn_navigator_page',{sessionId:id});if(request!==navRequest.current||activeSessionId.current!==id||!Array.isArray(page.entries))return;setNavError(false);setNavState(old=>old.id===id?{id,entries:mergeNav(page.entries,old.entries),cursor:page.olderCursor,hasOlder:page.hasOlder}:old)}catch(e){if(request===navRequest.current&&activeSessionId.current===id){setNavError(true);setError(errorText(e))}}}
 async function loadOlderNav(){const id=sessionId,request=navRequest.current,cursor=navState.cursor;if(!id||navLock.current||!cursor||!navState.hasOlder)return;navLock.current=true;setNavLoading(true);try{const page=await command<TurnNavigatorPage>('session_turn_navigator_page',{sessionId:id,beforeSeq:cursor});if(request!==navRequest.current||activeSessionId.current!==id)return;setNavState(old=>old.id===id&&old.cursor===cursor?{...old,entries:mergeNav(page.entries,old.entries),cursor:page.olderCursor,hasOlder:page.hasOlder}:old)}catch(e){if(request===navRequest.current)setError(errorText(e))}finally{if(request===navRequest.current){navLock.current=false;setNavLoading(false)}}}
 async function jumpToTurn(id:string){const task=++jumpRequest.current,session=sessionId;autoFollow.current=false;setJumping(true);setNavActive(id);
  try{while(!useChat.getState().events[session]?.some(e=>e.id===id)){if(task!==jumpRequest.current||activeSessionId.current!==session)return;if(!await loadOlderTimeline()){if(task===jumpRequest.current)setError('无法加载目标消息，请重试。');return}}
   const isCurrent=()=>task===jumpRequest.current&&activeSessionId.current===session;const target=await waitForMessage(()=>Array.from(conversationRef.current?.querySelectorAll<HTMLElement>('[data-message-id]')??[]).find(el=>el.dataset.messageId===id)??null,isCurrent);if(!isCurrent())return;if(!target){setError('目标消息尚未显示，请重试。');return}target.scrollIntoView?.({block:'start',behavior:'smooth'});
  }finally{if(task===jumpRequest.current)setJumping(false)}
 }
 function syncNavPosition(node:HTMLElement){const top=node.getBoundingClientRect().top;let current:string|null=null;node.querySelectorAll<HTMLElement>('[data-message-id]').forEach(el=>{if(el.getBoundingClientRect().top<=top+110)current=el.dataset.messageId??null});if(current)setNavActive(current)}
 return {navState,navLoading,navError,navActive,jumping,loadOlderNav,jumpToTurn,syncNavPosition,
  retry:()=>void loadLatestNav(sessionId,navRequest.current),cancelJump:()=>{++jumpRequest.current;setJumping(false)}};
}

function mergeNav(a:TurnNavigatorEntry[],b:TurnNavigatorEntry[]):TurnNavigatorEntry[]{return [...new Map([...a,...b].map(item=>[item.eventId,item])).values()].sort((x,y)=>BigInt(x.seq)<BigInt(y.seq)?-1:BigInt(x.seq)>BigInt(y.seq)?1:0)}
function navFromEvent(event:SessionEvent):TurnNavigatorEntry{const text=typeof event.data.content==='string'?event.data.content.trim().replace(/\s+/gu,' '):'';const chars=Array.from(new Intl.Segmenter(undefined,{granularity:'grapheme'}).segment(text||'用户输入'),part=>part.segment);return {eventId:event.id,seq:event.seq,preview:chars.slice(0,160).join(''),previewTruncated:chars.length>160}}
