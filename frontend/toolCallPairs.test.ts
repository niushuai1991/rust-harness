import {describe,expect,it} from 'vitest';
import {projectToolEvents} from './toolCallPairs';
import type {SessionEvent} from './types';

const event=(id:string,kind:string,data:Record<string,unknown>,sessionId='s'):SessionEvent=>({id,sessionId,kind,data,seq:id,createdAt:''});
const call=(id:string,callId:string,name:string,arguments_:Record<string,unknown>={})=>event(id,'tool.call',{id:callId,name,arguments:arguments_});
const result=(id:string,callId:string,name:string,content:unknown='ok')=>event(id,'tool.result',{callId,name,content});

describe('通用工具调用投影',()=>{
 it('按 callId 配对交错调用，其余事件透传',()=>{
  const a=call('1','a','read',{path:'a.ts'}),b=call('2','b','web_fetch',{url:'https://x'}),other=event('3','assistant.message',{}),rb=result('4','b','web_fetch'),ra=result('5','a','read');
  expect(projectToolEvents([a,b,other,rb,ra])).toEqual([
   {kind:'tool',key:'1',call:a,result:ra},
   {kind:'tool',key:'2',call:b,result:rb},
   {kind:'event',event:other}
  ]);
 });
 it('孤儿结果与孤儿调用保持独立节点',()=>{
  const r=result('2','a','read');
  expect(projectToolEvents([r])).toEqual([{kind:'tool',key:'2',result:r}]);
  const a=call('1','a','read');
  expect(projectToolEvents([a])).toEqual([{kind:'tool',key:'1',call:a}]);
  expect(projectToolEvents([a,r])).toEqual([{kind:'tool',key:'1',call:a,result:r}]);
 });
 it('重复或空 callId、跨会话事件不合并',()=>{
  const a=call('1','a','read'),duplicate=call('3','a','read'),r=result('2','a','read'),empty=call('4','','read');
  expect(projectToolEvents([a,duplicate,r,empty])).toHaveLength(4);
  expect(projectToolEvents([a,{...r,sessionId:'other'}])).toHaveLength(2);
 });
 it('结果早于调用的乱序结果不配对',()=>{
  // 与 shell 投影同规则：结果必须位于调用之后。
  const r=result('2','a','read'),a=call('3','a','read');
  expect(projectToolEvents([r,a])).toEqual([{kind:'tool',key:'2',result:r},{kind:'tool',key:'3',call:a}]);
 });
 it('shell 事件透传为普通事件，由专属投影处理',()=>{
  const a=call('1','a','shell',{command:'echo'}),r=result('2','a','shell','{}');
  expect(projectToolEvents([a,r])).toEqual([{kind:'event',event:a},{kind:'event',event:r}]);
 });
 it('缺少 callId/id 的旧事件兜底为孤儿节点，不参与配对',()=>{
  const orphanResult={...event('2','tool.result',{}),data:{name:'read',content:'x',path:'a.txt'}};
  const orphanCall={...event('3','tool.call',{}),data:{name:'read',arguments:{}}};
  expect(projectToolEvents([orphanResult,orphanCall])).toEqual([
   {kind:'tool',key:'2',result:orphanResult},{kind:'tool',key:'3',call:orphanCall}
  ]);
 });
 it('write/edit 与其余工具同样按 callId 配对',()=>{
  const a=call('1','a','edit',{path:'a.ts'});
  const r={...result('2','a','edit'),data:{callId:'a',name:'edit',content:'',diff:'--- a\n+++ b\n'}};
  expect(projectToolEvents([a,r])).toEqual([{kind:'tool',key:'1',call:a,result:r}]);
 });
});
