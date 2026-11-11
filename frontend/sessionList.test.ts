import {describe,expect,it} from 'vitest';
import {filterByTitle,groupByProject,groupByTime,relativeLabel,timeGroupOf} from './sessionList';
import type {SessionInfo,WorkspaceInfo} from './types';

// 固定"现在"：本地时区 2026-11-08（周日）12:00，避免测试随真实时间漂移。
const NOW=new Date(2026,10,8,12,0,0);
const at=(y:number,m:number,d:number,h:number,mi:number)=>new Date(y,m-1,d,h,mi).toISOString();
function session(overrides:Partial<SessionInfo>&{id:string}):SessionInfo{
 return {workspaceId:'ws-1',title:`会话 ${overrides.id}`,createdAt:at(2026,1,1,0,0),updatedAt:at(2026,11,8,9,0),parentId:null,...overrides};
}
function workspace(overrides:Partial<WorkspaceInfo>&{id:string}):WorkspaceInfo{
 return {sourcePath:`D:\\code\\${overrides.id}`,replicaPath:`C:\\replica\\${overrides.id}`,createdAt:at(2026,1,1,0,0),name:overrides.id,pinned:false,executionMode:'replica',worktrees:{},...overrides};
}

describe('timeGroupOf',()=>{
 it('按本地日历日划分今天/昨天/近7天/更早',()=>{
  expect(timeGroupOf(at(2026,11,8,0,1),NOW)).toBe(0);
  expect(timeGroupOf(at(2026,11,7,23,59),NOW)).toBe(1);
  expect(timeGroupOf(at(2026,11,2,0,0),NOW)).toBe(2);
  expect(timeGroupOf(at(2026,11,1,23,59),NOW)).toBe(3);
 });
 it('非法时间戳归入更早',()=>{
  expect(timeGroupOf('not-a-date',NOW)).toBe(3);
 });
});

describe('relativeLabel',()=>{
 it('今天按分钟/小时，昨天带时刻，近7天按星期，更早按日期',()=>{
  expect(relativeLabel(at(2026,11,8,12,0),NOW)).toBe('刚刚');
  expect(relativeLabel(at(2026,11,8,11,30),NOW)).toBe('30 分钟前');
  expect(relativeLabel(at(2026,11,8,9,0),NOW)).toBe('3 小时前');
  expect(relativeLabel(at(2026,11,7,18,24),NOW)).toBe('昨天 18:24');
  expect(relativeLabel(at(2026,11,4,10,0),NOW)).toBe('周三');
  expect(relativeLabel(at(2026,10,28,10,0),NOW)).toBe('10月28日');
  expect(relativeLabel(at(2025,10,28,10,0),NOW)).toBe('2025年10月28日');
 });
 it('非法时间戳返回空串',()=>{
  expect(relativeLabel('not-a-date',NOW)).toBe('');
 });
});

describe('filterByTitle',()=>{
 it('按标题大小写不敏感过滤，空查询原样返回',()=>{
  const list=[session({id:'a',title:'ChatGPT OAuth 流式闭环'}),session({id:'b',title:'SQLite 快照'})];
  expect(filterByTitle(list,'')).toBe(list);
  expect(filterByTitle(list,'sqlite').map(s=>s.id)).toEqual(['b']);
  expect(filterByTitle(list,'OAUTH').map(s=>s.id)).toEqual(['a']);
  expect(filterByTitle(list,'不存在')).toEqual([]);
 });
});

describe('groupByTime',()=>{
 it('按后端时间倒序聚合为连续分组，保持组内顺序',()=>{
  const list=[session({id:'a',updatedAt:at(2026,11,8,10,0)}),session({id:'b',updatedAt:at(2026,11,8,8,0)}),session({id:'c',updatedAt:at(2026,11,7,9,0)}),session({id:'d',updatedAt:at(2026,10,9,9,0)})];
  const groups=groupByTime(list,NOW);
  expect(groups.map(g=>g.group)).toEqual([0,1,3]);
  expect(groups[0].sessions.map(s=>s.id)).toEqual(['a','b']);
 });
 it('没有今天会话时仍保留今天空分组',()=>{
  const groups=groupByTime([session({id:'y',updatedAt:at(2026,11,7,9,0)}),session({id:'old',updatedAt:at(2026,10,9,9,0)})],NOW);
  expect(groups.map(g=>g.group)).toEqual([0,1,3]);
  expect(groups[0].sessions).toEqual([]);
 });
});

describe('groupByProject',()=>{
 it('有会话的工作区成组，置顶在前；两组按组内最新会话排序',()=>{
  const ws=[workspace({id:'ws-1'}),workspace({id:'ws-2'}),workspace({id:'ws-3',pinned:true})];
  const list=[
   session({id:'a',workspaceId:'ws-1',updatedAt:at(2026,11,8,10,0)}),
   session({id:'b',workspaceId:'ws-2',updatedAt:at(2026,11,8,9,0)}),
   session({id:'c',workspaceId:'ws-3',updatedAt:at(2026,11,7,9,0)}),
  ];
  const groups=groupByProject(list,ws);
  expect(groups.map(g=>g.workspace.id)).toEqual(['ws-3','ws-1','ws-2']);
  expect(groups[0].sessions.map(s=>s.id)).toEqual(['c']);
 });
 it('无会话的工作区不出现；未知工作区的会话忽略',()=>{
  const ws=[workspace({id:'ws-1'})];
  const list=[session({id:'a',workspaceId:'ws-1'}),session({id:'b',workspaceId:'ws-ghost'})];
  const groups=groupByProject(list,ws);
  expect(groups).toHaveLength(1);
  expect(groups[0].sessions.map(s=>s.id)).toEqual(['a']);
 });
});
