import {describe,expect,it} from 'vitest';
import {contextTokensFromEvents,formatTokens,usageTokensFromData} from './contextUsage';
import type {SessionEvent} from './types';

let seq=0;
function event(kind:string,data:Record<string,unknown>):SessionEvent{
 seq+=1;
 return {id:`e${seq}`,sessionId:'s1',seq:String(seq),kind,data,createdAt:'2026-11-08T00:00:00Z'};
}
function assistant(usage:unknown):SessionEvent{
 return event('assistant.message',{content:'ok',usage});
}

describe('contextTokensFromEvents',()=>{
 it('ChatCompletions usage：prompt+completion',()=>{
  const events=[assistant({prompt_tokens:100,completion_tokens:20})];
  expect(contextTokensFromEvents(events)).toBe(120);
 });
 it('Responses usage：input+output（input 已含 cached）',()=>{
  const events=[assistant({input_tokens:1000,output_tokens:200,input_tokens_details:{cached_tokens:800}})];
  expect(contextTokensFromEvents(events)).toBe(1200);
 });
 it('Anthropic usage：input+cache_read+cache_creation+output',()=>{
  const events=[assistant({input_tokens:100,output_tokens:50,cache_read_input_tokens:900,cache_creation_input_tokens:30})];
  expect(contextTokensFromEvents(events)).toBe(1080);
 });
 it('取最新一条 assistant.message 的 usage（Responses 每轮 input 即全量上下文）',()=>{
  const events=[assistant({input_tokens:1000,output_tokens:100}),assistant({input_tokens:5000,output_tokens:300})];
  expect(contextTokensFromEvents(events)).toBe(5300);
 });
 it('压缩点之前的 usage 作废：compaction 后取其后最新 usage',()=>{
  const events=[
   assistant({input_tokens:9000,output_tokens:100}),
   event('compaction.summary',{throughSeq:'2'}),
   assistant({input_tokens:800,output_tokens:50}),
  ];
  expect(contextTokensFromEvents(events)).toBe(850);
 });
 it('压缩后尚无新 usage → null',()=>{
  const events=[assistant({input_tokens:9000,output_tokens:100}),event('compaction.summary',{throughSeq:'2'})];
  expect(contextTokensFromEvents(events)).toBeNull();
 });
 it('无任何带 usage 的事件 → null',()=>{
  expect(contextTokensFromEvents([event('user.message',{content:'hi'})])).toBeNull();
  expect(contextTokensFromEvents([])).toBeNull();
 });
 it('assistant.message 缺 usage（失败 attempt 语义）时向前回退',()=>{
  const events=[assistant({prompt_tokens:100,completion_tokens:10}),assistant({content:'failed'})];
  expect(contextTokensFromEvents(events)).toBe(110);
 });
 it('usageTokensFromData：usage 非对象或无有效数字 → null',()=>{
  expect(usageTokensFromData({usage:'x'})).toBeNull();
  expect(usageTokensFromData({usage:{input_tokens:'200'}})).toBeNull();
  expect(usageTokensFromData({})).toBeNull();
 });
});

describe('formatTokens',()=>{
 it('按量级缩写',()=>{
  expect(formatTokens(0)).toBe('0');
  expect(formatTokens(950)).toBe('950');
  expect(formatTokens(62_100)).toBe('62.1k');
  expect(formatTokens(200_000)).toBe('200k');
  expect(formatTokens(1_250_000)).toBe('1.3M');
 });
});
