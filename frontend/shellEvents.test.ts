import {describe,expect,it} from 'vitest';
import {parseShellResult,projectShellEvents} from './shellEvents';
import type {SessionEvent} from './types';

const event=(id:string,kind:string,data:Record<string,unknown>,sessionId='s'):SessionEvent=>({id,sessionId,kind,data,seq:id,createdAt:''});
const call=(id:string,callId:string)=>event(id,'tool.call',{id:callId,name:'shell',arguments:{command:`echo ${callId}`}});
const result=(id:string,callId:string)=>event(id,'tool.result',{callId,name:'shell',content:'{}'});

describe('Shell 时间线投影',()=>{
 it('按 ID 匹配交错并发调用，保留其他事件位置',()=>{
  const a=call('1','a'),b=call('2','b'),other=event('3','assistant.message',{}),rb=result('4','b'),ra=result('5','a');
  expect(projectShellEvents([a,b,other,rb,ra])).toEqual([
   {kind:'shell',key:'1',call:a,result:ra},{kind:'shell',key:'2',call:b,result:rb},{kind:'event',event:other}
  ]);
 });
 it('缺失或重复 ID、不同会话的事件保持独立，补齐历史后可配对',()=>{
  const a=call('1','a'),r=result('2','a');
  expect(projectShellEvents([r])).toEqual([{kind:'shell',key:'2',result:r}]);
  expect(projectShellEvents([a,r])).toEqual([{kind:'shell',key:'1',call:a,result:r}]);
  const duplicate=call('3','a'),empty=call('4','');
  expect(projectShellEvents([a,duplicate,r,empty])).toHaveLength(4);
  expect(projectShellEvents([a,{...r,sessionId:'other'}])).toHaveLength(2);
 });
});
describe('Shell 结果解析',()=>{
 it('区分标准输出、标准错误、退出码及空输出',()=>{
  expect(parseShellResult('{"stdout":"ok\\n","stderr":"warning","exit_code":0}')).toEqual({kind:'output',stdout:'ok\n',stderr:'warning',exitCode:0});
  expect(parseShellResult('{"stdout":"","stderr":"","exit_code":1}')).toEqual({kind:'output',stdout:'',stderr:'',exitCode:1});
 });
 it.each(['拒绝执行','{"other":1}','{"stdout":"partial','{"stdout":4,"stderr":"","exit_code":0}'])('解析失败时保留原文 %s',value=>{
  expect(parseShellResult(value)).toEqual({kind:'raw',text:value});
 });
});
