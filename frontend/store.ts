import {create} from 'zustand';
import type {SessionEvent,LiveEvent,TimelinePage} from './types';

const MAX_TIMELINE_PAGES=3,MAX_TIMELINE_SESSIONS=3,MAX_EVENTS_PER_PAGE=200;
export interface Stream {requestId:string;text:string;reasoning:string;seen:string[]}
export interface TimelineState {olderCursor:string|null;hasOlder:boolean;throughSeq:string;pagesLoaded:number;reachedPageLimit:boolean;pages:SessionEvent[][]}
interface State {
 events:Record<string,SessionEvent[]>;timelines:Record<string,TimelineState>;timelineLru:string[];streams:Record<string,Stream>;statuses:Record<string,string>;
 replace:(id:string,events:SessionEvent[])=>void;replaceTimelinePage:(id:string,page:TimelinePage)=>void;prependTimelinePage:(id:string,page:TimelinePage)=>void;activateTimeline:(id:string)=>void;timeline:(id:string)=>TimelineState|undefined;
 receive:(event:LiveEvent)=>void;clearStream:(id:string)=>void
}
export const useChat=create<State>((set,get)=>({events:{},timelines:{},timelineLru:[],streams:{},statuses:{},
 replace:(id,events)=>set(state=>({events:{...state.events,[id]:mergeEvents(events,state.events[id]??[])}})),
 replaceTimelinePage:(id,page)=>set(state=>storeTimeline(state,id,timelineFromPage(page))),
 prependTimelinePage:(id,page)=>set(state=>{
  const current=state.timelines[id];if(!current||current.reachedPageLimit)return state;
  const pages=[page.events,...current.pages].slice(0,MAX_TIMELINE_PAGES);
  return storeTimeline(state,id,{...current,pages,olderCursor:page.olderCursor,hasOlder:page.hasOlder,pagesLoaded:pages.length,reachedPageLimit:pages.length===MAX_TIMELINE_PAGES&&page.hasOlder});
 }),
 activateTimeline:(id)=>set(state=>storeTimeline(state,id,state.timelines[id]??emptyTimeline())),
 timeline:(id)=>get().timelines[id],
 clearStream:(id)=>set(state=>{const streams={...state.streams};delete streams[id];return {streams}}),
 receive:(event)=>set(state=>{
  if(event.type==='durable'){
   const id=event.event.sessionId,streams={...state.streams};
   if(event.event.kind==='assistant.message'||event.event.kind==='message.assistant')delete streams[id];
   const current=state.timelines[id];
   if(!current)return {events:{...state.events,[id]:mergeEvents(state.events[id]??[],[event.event])},streams};
   const pages=current.pages.length?current.pages.map((page,index)=>index===current.pages.length-1?trimPage(mergeEvents(page,[event.event])):page):[[event.event]];
   return {...storeTimeline(state,id,{...current,pages,throughSeq:maxSeq(current.throughSeq,event.event.seq)}),streams};
  }
  if(event.type==='status')return {statuses:{...state.statuses,[event.sessionId]:event.status}};
  const old=state.streams[event.sessionId],stream=old?.requestId===event.requestId?old:{requestId:event.requestId,text:'',reasoning:'',seen:[]};
  const key=`${event.reasoning}:${event.index}`;if(stream.seen.includes(key))return state;
  return {streams:{...state.streams,[event.sessionId]:{...stream,[event.reasoning?'reasoning':'text']:stream[event.reasoning?'reasoning':'text']+event.text,seen:[...stream.seen,key]}}};
 })
}));

function timelineFromPage(page:TimelinePage):TimelineState {return {olderCursor:page.olderCursor,hasOlder:page.hasOlder,throughSeq:page.throughSeq,pagesLoaded:1,reachedPageLimit:false,pages:[trimPage(page.events)]}}
function emptyTimeline():TimelineState {return {olderCursor:null,hasOlder:false,throughSeq:'0',pagesLoaded:0,reachedPageLimit:false,pages:[]}}
function storeTimeline(state:State,id:string,timeline:TimelineState){
 const lru=[id,...state.timelineLru.filter(item=>item!==id)],events={...state.events,[id]:flattenPages(timeline.pages)},timelines={...state.timelines,[id]:timeline};
 for(const evicted of lru.slice(MAX_TIMELINE_SESSIONS)){delete events[evicted];delete timelines[evicted]}
 return {events,timelines,timelineLru:lru.slice(0,MAX_TIMELINE_SESSIONS)};
}
function flattenPages(pages:SessionEvent[][]){return pages.reduce((all,page)=>mergeEvents(all,page),[] as SessionEvent[])}
function trimPage(events:SessionEvent[]){return events.length>MAX_EVENTS_PER_PAGE?events.slice(-MAX_EVENTS_PER_PAGE):events}
function maxSeq(a:string,b:string){return BigInt(a)>=BigInt(b)?a:b}
function mergeEvents(a:SessionEvent[],b:SessionEvent[]){return [...new Map([...a,...b].map(event=>[event.id,event])).values()].sort((x,y)=>BigInt(x.seq)<BigInt(y.seq)?-1:BigInt(x.seq)>BigInt(y.seq)?1:0)}
