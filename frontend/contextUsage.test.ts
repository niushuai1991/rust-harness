import {describe,expect,it} from 'vitest';
import {cacheHitFromEvents,contextTokensFromEvents,formatHitRate,formatTokens,usageTokensFromData} from './contextUsage';
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

// 缓存命中率（口径对齐 zcode mainTurnCacheHitAggregate：Σcache_read / Σtotal input，
// 分母为完整输入含 cached 与 cache_creation；累计范围与上下文占用一致，压缩点止步）。
describe('cacheHitFromEvents',()=>{
 it('Anthropic：累加多条，cache_creation 进分母不进分子',()=>{
  const events=[
   assistant({input_tokens:100,output_tokens:50,cache_read_input_tokens:900,cache_creation_input_tokens:30}),
   assistant({input_tokens:200,output_tokens:80,cache_read_input_tokens:400,cache_creation_input_tokens:0}),
  ];
  expect(cacheHitFromEvents(events)).toEqual({cacheRead:1300,totalInput:1630});
 });
 it('ChatCompletions：cached_tokens 进分子，prompt_tokens 为分母',()=>{
  const events=[assistant({prompt_tokens:1000,completion_tokens:20,prompt_tokens_details:{cached_tokens:800}})];
  expect(cacheHitFromEvents(events)).toEqual({cacheRead:800,totalInput:1000});
 });
 it('Responses：cached_tokens 进分子，input_tokens 为分母',()=>{
  const events=[assistant({input_tokens:1000,output_tokens:200,input_tokens_details:{cached_tokens:640}})];
  expect(cacheHitFromEvents(events)).toEqual({cacheRead:640,totalInput:1000});
 });
 it('跨多条请求累计',()=>{
  const events=[
   assistant({input_tokens:1000,output_tokens:100,input_tokens_details:{cached_tokens:800}}),
   assistant({input_tokens:2000,output_tokens:300,input_tokens_details:{cached_tokens:1500}}),
  ];
  expect(cacheHitFromEvents(events)).toEqual({cacheRead:2300,totalInput:3000});
 });
 it('压缩点止步：压缩前 usage 不计',()=>{
  const events=[
   assistant({input_tokens:9000,output_tokens:100,input_tokens_details:{cached_tokens:8000}}),
   event('compaction.summary',{throughSeq:'2'}),
   assistant({input_tokens:800,output_tokens:50,input_tokens_details:{cached_tokens:600}}),
  ];
  expect(cacheHitFromEvents(events)).toEqual({cacheRead:600,totalInput:800});
 });
 it('压缩后尚无新 usage → null',()=>{
  const events=[assistant({input_tokens:900,output_tokens:10,cache_read_input_tokens:800}),event('compaction.summary',{throughSeq:'2'})];
  expect(cacheHitFromEvents(events)).toBeNull();
 });
 it('缺缓存字段的 usage 整条跳过（不进分子分母）',()=>{
  const events=[
   assistant({prompt_tokens:500,completion_tokens:20}),
   assistant({input_tokens:1000,output_tokens:100,input_tokens_details:{cached_tokens:800}}),
  ];
  expect(cacheHitFromEvents(events)).toEqual({cacheRead:800,totalInput:1000});
 });
 it('cache_read 字段存在但为 0（冷缓存）→ 如实计入 0 命中',()=>{
  const events=[assistant({input_tokens:1000,output_tokens:50,cache_read_input_tokens:0,cache_creation_input_tokens:200})];
  expect(cacheHitFromEvents(events)).toEqual({cacheRead:0,totalInput:1200});
 });
 it('全部缺缓存字段 → null',()=>{
  expect(cacheHitFromEvents([assistant({prompt_tokens:100,completion_tokens:10})])).toBeNull();
  expect(cacheHitFromEvents([assistant({input_tokens:100,output_tokens:10})])).toBeNull();
 });
 it('无任何 usage / 空事件流 → null',()=>{
  expect(cacheHitFromEvents([event('user.message',{content:'hi'})])).toBeNull();
  expect(cacheHitFromEvents([])).toBeNull();
 });
 it('totalInput 无法解析（缺输入字段）→ null',()=>{
  expect(cacheHitFromEvents([assistant({cache_read_input_tokens:100})])).toBeNull();
 });
});

describe('formatHitRate',()=>{
 it('整数百分比不带小数',()=>{expect(formatHitRate(800,1000)).toBe('80%')});
 it('最多一位小数',()=>{expect(formatHitRate(1,3)).toBe('33.3%')});
 it('全部命中为 100%',()=>{expect(formatHitRate(500,500)).toBe('100%')});
});
