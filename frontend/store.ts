import {create} from 'zustand';
import type {SessionEvent,LiveEvent} from './types';
export interface Stream {requestId:string;text:string;reasoning:string;seen:string[]}
interface State {events:Record<string,SessionEvent[]>;streams:Record<string,Stream>;statuses:Record<string,string>;replace:(id:string,events:SessionEvent[])=>void;receive:(event:LiveEvent)=>void;clearStream:(id:string)=>void}
export const useChat = create<State>((set)=>({events:{},streams:{},statuses:{},
 replace:(id,events)=>set(state=>({events:{...state.events,[id]:mergeEvents(events,state.events[id]??[])}})),
 clearStream:(id)=>set(state=>{const streams={...state.streams};delete streams[id];return {streams}}),
 receive:(event)=>set(state=>{
  if(event.type==='durable') {
   const id=event.event.sessionId; const streams={...state.streams};
   if(event.event.kind==='assistant.message'||event.event.kind==='message.assistant') delete streams[id];
   return {events:{...state.events,[id]:mergeEvents(state.events[id]??[],[event.event])},streams};
  }
  if(event.type==='status')return {statuses:{...state.statuses,[event.sessionId]:event.status}};
  const old=state.streams[event.sessionId]; const stream=old?.requestId===event.requestId?old:{requestId:event.requestId,text:'',reasoning:'',seen:[]};
  const key=`${event.reasoning}:${event.index}`; if(stream.seen.includes(key))return state;
  return {streams:{...state.streams,[event.sessionId]:{...stream,[event.reasoning?'reasoning':'text']:stream[event.reasoning?'reasoning':'text']+event.text,seen:[...stream.seen,key]}}};
 })
}));
function mergeEvents(a:SessionEvent[],b:SessionEvent[]) {return [...new Map([...a,...b].map(e=>[e.id,e])).values()].sort((x,y)=>BigInt(x.seq)<BigInt(y.seq)?-1:BigInt(x.seq)>BigInt(y.seq)?1:0)}
