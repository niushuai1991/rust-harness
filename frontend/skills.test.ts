import {describe,expect,it} from 'vitest';
import {filterSkillRows,skillRowsFrom,skillTokenUnderCursor} from './skills';
import type {SkillInfo} from './types';

const info=(over:Partial<SkillInfo>):SkillInfo=>({name:'review',description:'代码审查',path:'.agents/skills/review/SKILL.md',scope:'project',enabled:true,brokenReason:null,...over});

describe('技能候选派生',()=>{
 it('仅启用且未 broken 的技能进入 $ 补全候选',()=>{
  const rows=skillRowsFrom([info({}),info({name:'bad',brokenReason:'name 非法'}),info({name:'off',enabled:false})]);
  expect(rows.map(r=>r.name)).toEqual(['review']);
 });
 it('无描述技能补全候选带空描述',()=>{
  expect(skillRowsFrom([info({description:null})])).toEqual([{name:'review',description:''}]);
 });
});

describe('$token 提取',()=>{
 it('光标处 $token 返回区间',()=>{
  const text='请 $rev 继续';
  expect(skillTokenUnderCursor(text,6)).toEqual({token:'$rev',start:2,end:6});
 });
 it('消息中间的 $token 也可提取',()=>{
  expect(skillTokenUnderCursor('a $deploy b',5)).toEqual({token:'$deploy',start:2,end:9});
 });
 it('非 $ token 或越过 token 末尾返回 null',()=>{
  expect(skillTokenUnderCursor('普通文本',4)).toBeNull();
  expect(skillTokenUnderCursor('$rev x',6)).toBeNull();
  expect(skillTokenUnderCursor('邮箱 a$x',4)).toBeNull();
  expect(skillTokenUnderCursor('邮箱 $x',5)).toEqual({token:'$x',start:3,end:5});
 });
 it('$ 后无名字也是合法 token（触发空候选弹窗由调用方过滤）',()=>{
  expect(skillTokenUnderCursor('$',1)).toEqual({token:'$',start:0,end:1});
 });
});

describe('候选过滤',()=>{
 const rows=[{name:'review',description:'代码审查流程'},{name:'deploy',description:'部署应用'}];
 it('大小写不敏感匹配名称与描述',()=>{
  expect(filterSkillRows(rows,'DEP')).toEqual([rows[1]]);
  expect(filterSkillRows(rows,'审查')).toEqual([rows[0]]);
 });
 it('空查询返回全量，无命中为空',()=>{
  expect(filterSkillRows(rows,'')).toEqual(rows);
  expect(filterSkillRows(rows,'zzz')).toEqual([]);
 });
});
