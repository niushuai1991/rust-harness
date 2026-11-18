import {describe,expect,it} from 'vitest';
import {filterByTitle,formatFullTime,groupByProject,groupByTime,relativeLabel,subagentsOf,timeGroupOf,upsertSession,withoutSubagents} from './sessionList';
import type {SessionInfo,WorkspaceInfo} from './types';

// 固定"现在"：本地时区 2026-11-08（周日）12:00，避免测试随真实时间漂移。
const NOW=new Date(2026,10,8,12,0,0);
const at=(y:number,m:number,d:number,h:number,mi:number)=>new Date(y,m-1,d,h,mi).toISOString();
function session(overrides:Partial<SessionInfo>&{id:string}):SessionInfo{
 return {workspaceId:'ws-1',title:`会话 ${overrides.id}`,createdAt:at(2026,1,1,0,0),updatedAt:at(2026,11,8,9,0),parentId:null,...overrides};
}
function workspace(overrides:Partial<WorkspaceInfo>&{id:string}):WorkspaceInfo{
 return {sourcePath:`D:\\code\\${overrides.id}`,replicaPath:`C:\\replica\\${overrides.id}`,createdAt:at(2026,1,1,0,0),name:overrides.id,pinned:false,hidden:false,executionMode:'replica',worktrees:{},...overrides};
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

describe('formatFullTime',()=>{
 it('输出本地时区的完整日期与时刻，分钟补零',()=>{
  expect(formatFullTime(at(2026,11,8,9,5))).toBe('2026年11月8日 09:05');
  expect(formatFullTime(at(2025,1,13,21,47))).toBe('2025年1月13日 21:47');
 });
 it('非法时间戳返回空串',()=>{
  expect(formatFullTime('not-a-date')).toBe('');
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

describe('withoutSubagents',()=>{
 it('过滤子代理会话，保留主会话原顺序',()=>{
  const list=[session({id:'a'}),session({id:'c1',parentId:'a',title:'依赖梳理子任务'}),session({id:'b'}),session({id:'c2',parentId:'a'})];
  expect(withoutSubagents(list).map(s=>s.id)).toEqual(['a','b']);
 });
 it('全部为子代理会话时返回空数组',()=>{
  const list=[session({id:'c1',parentId:'p'}),session({id:'c2',parentId:'p'})];
  expect(withoutSubagents(list)).toEqual([]);
 });
 it('空列表与无 parentId 会话原样保留',()=>{
  expect(withoutSubagents([])).toEqual([]);
  const list=[session({id:'a'})];
  expect(withoutSubagents(list)).toEqual(list);
 });
});

describe('subagentsOf',()=>{
 it('只返回当前会话的子代理，保持原顺序',()=>{
  const list=[session({id:'c1',parentId:'p1'}),session({id:'c2',parentId:'p2'}),session({id:'c3',parentId:'p1'}),session({id:'a'})];
  expect(subagentsOf(list,'p1').map(s=>s.id)).toEqual(['c1','c3']);
 });
 it('会话未创建（无 id）时返回空数组',()=>{
  const list=[session({id:'c1',parentId:'p1'})];
  expect(subagentsOf(list,null)).toEqual([]);
  expect(subagentsOf(list,'')).toEqual([]);
 });
 it('无匹配父会话或空列表时返回空数组',()=>{
  expect(subagentsOf([session({id:'c1',parentId:'p1'})],'p2')).toEqual([]);
  expect(subagentsOf([session({id:'a'})],'p1')).toEqual([]);
  expect(subagentsOf([],'p1')).toEqual([]);
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
 it('缺省无会话的工作区不出现；未知工作区的会话忽略',()=>{
  const ws=[workspace({id:'ws-1'})];
  const list=[session({id:'a',workspaceId:'ws-1'}),session({id:'b',workspaceId:'ws-ghost'})];
  const groups=groupByProject(list,ws);
  expect(groups).toHaveLength(1);
  expect(groups[0].sessions.map(s=>s.id)).toEqual(['a']);
 });
 it('includeEmpty 时空项目也成组并排在有会话项目之后，空组间按名称排序',()=>{
  const ws=[workspace({id:'ws-1'}),workspace({id:'ws-3',name:'zzz'}),workspace({id:'ws-2',name:'aaa'})];
  const list=[session({id:'a',workspaceId:'ws-1',updatedAt:at(2026,11,8,10,0)})];
  const groups=groupByProject(list,ws,true);
  expect(groups.map(g=>g.workspace.id)).toEqual(['ws-1','ws-2','ws-3']);
  expect(groups[1].sessions).toEqual([]);
  expect(groups[2].sessions).toEqual([]);
 });
 it('includeEmpty 时置顶的空项目仍排在最前',()=>{
  const ws=[workspace({id:'ws-1'}),workspace({id:'ws-2',pinned:true})];
  const list=[session({id:'a',workspaceId:'ws-1'})];
  const groups=groupByProject(list,ws,true);
  expect(groups.map(g=>g.workspace.id)).toEqual(['ws-2','ws-1']);
 });
});

describe('upsertSession',()=>{
 it('新会话按 updatedAt 倒序插入到正确位置',()=>{
  const list=[session({id:'a',updatedAt:at(2026,11,8,10,0)}),session({id:'b',updatedAt:at(2026,11,8,8,0)})];
  const next=upsertSession(list,session({id:'new',updatedAt:at(2026,11,8,9,0)}));
  expect(next.map(s=>s.id)).toEqual(['a','new','b']);
 });
 it('比所有会话都新时置顶',()=>{
  const list=[session({id:'a',updatedAt:at(2026,11,8,10,0)})];
  expect(upsertSession(list,session({id:'new',updatedAt:at(2026,11,8,11,0)})).map(s=>s.id)).toEqual(['new','a']);
 });
 it('已有会话更新时去重并按新 updatedAt 重排',()=>{
  const list=[session({id:'a',updatedAt:at(2026,11,8,10,0)}),session({id:'b',updatedAt:at(2026,11,8,8,0)})];
  const next=upsertSession(list,session({id:'b',updatedAt:at(2026,11,8,11,0),title:'更新后的标题'}));
  expect(next.map(s=>s.id)).toEqual(['b','a']);
  expect(next[0].title).toBe('更新后的标题');
 });
});
describe('groupByProject 无项目组',()=>{
 it('哨兵空串会话归入「无项目」组，固定排在全部项目组之后',()=>{
  const ws=[workspace({id:'ws-1'}),workspace({id:'ws-2',pinned:true})];
  const list=[
   session({id:'a',workspaceId:'ws-1',updatedAt:at(2026,11,8,10,0)}),
   session({id:'n1',workspaceId:'',updatedAt:at(2026,11,8,11,0)}),
  ];
  const groups=groupByProject(list,ws,true);
  expect(groups.map(g=>g.workspace.id)).toEqual(['ws-2','ws-1','']);
  expect(groups[2].workspace.name).toBe('无项目');
  expect(groups[2].sessions.map(s=>s.id)).toEqual(['n1']);
 });
 it('includeEmpty 时不出现空的无项目组',()=>{
  const ws=[workspace({id:'ws-1'})];
  const groups=groupByProject([session({id:'a',workspaceId:'ws-1'})],ws,true);
  expect(groups.map(g=>g.workspace.id)).toEqual(['ws-1']);
 });
 it('搜索过滤场景（includeEmpty=false）：无项目会话有匹配则成组，无匹配不出现',()=>{
  const ws=[workspace({id:'ws-1'})];
  const groups=groupByProject([session({id:'n1',workspaceId:'',title:'独立讨论'})],ws);
  expect(groups.map(g=>g.workspace.id)).toEqual(['']);
  expect(groups[0].workspace.name).toBe('无项目');
  expect(groupByProject([session({id:'a',workspaceId:'ws-1'})],ws)).toHaveLength(1);
 });
});
