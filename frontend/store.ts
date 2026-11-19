import {create} from 'zustand';
import type {SessionEvent,LiveEvent,TimelinePage,TimelineTurn} from './types';

const MAX_TIMELINE_SESSIONS=3,MAX_EVENTS_PER_PAGE=200;
export interface Stream {requestId:string;text:string;reasoning:string;seen:string[]}
export interface TimelineState {olderCursor:string|null;hasOlder:boolean;throughSeq:string;pagesLoaded:number;loadedOlder:boolean;pages:SessionEvent[][];turns:TimelineTurn[]}
interface State {
 events:Record<string,SessionEvent[]>;timelines:Record<string,TimelineState>;timelineLru:string[];streams:Record<string,Stream>;statuses:Record<string,string>;
 replace:(id:string,events:SessionEvent[])=>void;replaceTimelinePage:(id:string,page:TimelinePage)=>void;prependTimelinePage:(id:string,page:TimelinePage)=>void;activateTimeline:(id:string)=>void;timeline:(id:string)=>TimelineState|undefined;
 receive:(event:LiveEvent)=>void;clearStream:(id:string)=>void
}
export const useChat=create<State>((set,get)=>({events:{},timelines:{},timelineLru:[],streams:{},statuses:{},
 replace:(id,events)=>set(state=>({events:{...state.events,[id]:mergeEvents(events,state.events[id]??[])}})),
 replaceTimelinePage:(id,page)=>set(state=>{
  const newer=(state.events[id]??[]).filter(event=>BigInt(event.seq)>BigInt(page.throughSeq));
  return storeTimeline(state,id,timelineFromPage(page,newer));
 }),
 // 历史不限页数：用户加载多少页就保留多少页，直到切出会话。
 prependTimelinePage:(id,page)=>set(state=>{
  const current=state.timelines[id];if(!current)return state;
  const pages=[page.events,...current.pages];
  return storeTimeline(state,id,{...current,pages,turns:[...new Map([...(page.turns??[]),...current.turns].map(turn=>[turn.startSeq,turn])).values()],olderCursor:page.olderCursor,hasOlder:page.hasOlder,pagesLoaded:pages.length,loadedOlder:true});
 }),
 activateTimeline:(id)=>set(state=>storeTimeline(state,id,state.timelines[id]??emptyTimeline())),
 timeline:(id)=>get().timelines[id],
 clearStream:(id)=>set(state=>{const streams={...state.streams};delete streams[id];return {streams}}),
 receive:(event)=>set(state=>{
  if(event.type==='durable'){
   const id=event.event.sessionId,streams={...state.streams};
   if(event.event.kind==='assistant.message'||event.event.kind==='message.assistant')delete streams[id];
   const current=state.timelines[id];
   if(!current)return {streams};
   const pages=appendDurable(current.pages,event.event);
   return {...storeTimeline(state,id,{...current,pages,throughSeq:maxSeq(current.throughSeq,event.event.seq)}),streams};
  }
  if(event.type==='status')return {statuses:{...state.statuses,[event.sessionId]:event.status}};
  const old=state.streams[event.sessionId],stream=old?.requestId===event.requestId?old:{requestId:event.requestId,text:'',reasoning:'',seen:[]};
  const key=`${event.reasoning}:${event.index}`;if(stream.seen.includes(key))return state;
  return {streams:{...state.streams,[event.sessionId]:{...stream,[event.reasoning?'reasoning':'text']:stream[event.reasoning?'reasoning':'text']+event.text,seen:[...stream.seen,key]}}};
 })
}));

function timelineFromPage(page:TimelinePage,newer:SessionEvent[]=[]):TimelineState {
 const all=mergeEvents(page.events,newer),pages=chunkEvents(all);
 return {olderCursor:page.olderCursor,hasOlder:page.hasOlder,throughSeq:all.reduce((seq,event)=>maxSeq(seq,event.seq),page.throughSeq),pagesLoaded:pages.length,loadedOlder:false,pages,turns:page.turns??[]}
}
function emptyTimeline():TimelineState {return {olderCursor:null,hasOlder:false,throughSeq:'0',pagesLoaded:0,loadedOlder:false,pages:[],turns:[]}}
function storeTimeline(state:State,id:string,timeline:TimelineState){
 const lru=[id,...state.timelineLru.filter(item=>item!==id)],events={...state.events,[id]:flattenPages(timeline.pages)},timelines={...state.timelines,[id]:timeline};
 for(const evicted of lru.slice(MAX_TIMELINE_SESSIONS)){delete events[evicted];delete timelines[evicted]}
 return {events,timelines,timelineLru:lru.slice(0,MAX_TIMELINE_SESSIONS)};
}
function flattenPages(pages:SessionEvent[][]){return pages.reduce((all,page)=>mergeEvents(all,page),[] as SessionEvent[])}
// 满页时另起一页而不是裁掉最早事件：实时追加不能丢任何已到达的会话内容。
function appendDurable(pages:SessionEvent[][],event:SessionEvent):SessionEvent[][] {
 if(!pages.length)return [[event]];
 const last=mergeEvents(pages[pages.length-1],[event]);
 if(last.length>MAX_EVENTS_PER_PAGE)return [...pages,[event]];
 return [...pages.slice(0,-1),last];
}
function chunkEvents(events:SessionEvent[]):SessionEvent[][] {
 const chunks:SessionEvent[][]=[];
 for(let index=0;index<events.length;index+=MAX_EVENTS_PER_PAGE)chunks.push(events.slice(index,index+MAX_EVENTS_PER_PAGE));
 return chunks;
}
function maxSeq(a:string,b:string){return BigInt(a)>=BigInt(b)?a:b}
function mergeEvents(a:SessionEvent[],b:SessionEvent[]){return [...new Map([...a,...b].map(event=>[event.id,event])).values()].sort((x,y)=>BigInt(x.seq)<BigInt(y.seq)?-1:BigInt(x.seq)>BigInt(y.seq)?1:0)}
