import {beforeEach,describe,expect,it} from 'vitest';
import {useChat} from './store';
import type {SessionEvent,TimelinePage} from './types';

function event(sessionId:string,seq:number):SessionEvent {
 return {id:`${sessionId}-${seq}`,sessionId,seq:String(seq),kind:'assistant.message',data:{content:`${sessionId}-${seq}`},createdAt:''};
}

function page(sessionId:string,from:number,to:number,olderCursor:string|null,hasOlder:boolean):TimelinePage {
 return {sessionId,events:Array.from({length:to-from+1},(_,index)=>event(sessionId,from+index)),olderCursor,throughSeq:String(to),hasOlder};
}

beforeEach(()=>useChat.setState({events:{},timelines:{},timelineLru:[],streams:{},statuses:{}}));

describe('timeline page cache',()=>{
 it('replaces the latest page and prepends at most three older pages',()=>{
  const state=useChat.getState();
  state.replaceTimelinePage('a',page('a',161,240,'161',true));
  state.prependTimelinePage('a',page('a',81,160,'81',true));
  state.prependTimelinePage('a',page('a',1,80,'1',true));

  expect(useChat.getState().events.a.map(item=>item.seq)).toEqual(Array.from({length:240},(_,index)=>String(index+1)));
  expect(useChat.getState().timeline('a')).toMatchObject({olderCursor:'1',pagesLoaded:3,reachedPageLimit:true});
 });

 it('releases the least recently used inactive session after a fourth timeline is opened',()=>{
  for(const sessionId of ['a','b','c','d'])useChat.getState().replaceTimelinePage(sessionId,page(sessionId,1,1,null,false));

  expect(useChat.getState().events.a).toBeUndefined();
  expect(useChat.getState().timeline('a')).toBeUndefined();
  expect(useChat.getState().events.d.map(item=>item.id)).toEqual(['d-1']);
 });

 it('does not recreate an unbounded event cache for an LRU-evicted session',()=>{
  for(const sessionId of ['a','b','c','d'])useChat.getState().replaceTimelinePage(sessionId,page(sessionId,1,1,null,false));

  useChat.getState().receive({type:'durable',event:event('a',2)});

  expect(useChat.getState().events.a).toBeUndefined();
  expect(useChat.getState().timeline('a')).toBeUndefined();
 });

 it('retains durable events that arrive after a latest-page snapshot begins',()=>{
  const state=useChat.getState(),snapshot=page('a',1,1,null,false);
  state.replaceTimelinePage('a',snapshot);
  state.receive({type:'durable',event:event('a',2)});
  state.replaceTimelinePage('a',snapshot);

  expect(useChat.getState().events.a.map(item=>item.seq)).toEqual(['1','2']);
  expect(useChat.getState().timeline('a')?.throughSeq).toBe('2');
 });
});
