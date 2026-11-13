import {describe,expect,it} from 'vitest';
import {GOAL_PROMPT,INIT_PROMPT,SLASH_COMMANDS,commandTokenUnderCursor,expandSlashPrompt,filterSlashCommands} from './slashCommands';

describe('斜杠命令注册表',()=>{
 it('包含桌面端可执行子集且无重复',()=>{
  const names=SLASH_COMMANDS.map(spec=>spec.name);
  expect(names).toEqual(['clear','compact','init','plan','goal','model','diff','status','help']);
  expect(new Set(names).size).toBe(names.length);
 });
 it('过滤：exact 优先于 prefix，按注册顺序，容忍前导斜杠',()=>{
  expect(filterSlashCommands('model')).toEqual([SLASH_COMMANDS[5]]);
  expect(filterSlashCommands('mo')).toEqual([SLASH_COMMANDS[5]]);
  expect(filterSlashCommands('/cl')).toEqual([SLASH_COMMANDS[0]]);
  expect(filterSlashCommands('s')).toEqual([SLASH_COMMANDS[7]]);
  expect(filterSlashCommands('pl')).toEqual([SLASH_COMMANDS[3]]);
  expect(filterSlashCommands('go')).toEqual([SLASH_COMMANDS[4]]);
  expect(filterSlashCommands('')).toEqual(SLASH_COMMANDS);
  expect(filterSlashCommands('zzz')).toEqual([]);
 });
 it('光标不在首命令 token 内时返回 null',()=>{
  expect(commandTokenUnderCursor('/cl',3)).toBe('/cl');
  expect(commandTokenUnderCursor('/cl',2)).toBe('/cl');
  expect(commandTokenUnderCursor('/cl 摘要',2)).toBe('/cl');
  expect(commandTokenUnderCursor('/cl 摘要',4)).toBeNull();
  expect(commandTokenUnderCursor('/cl ',4)).toBeNull();
  expect(commandTokenUnderCursor('修复登录页',2)).toBeNull();
  expect(commandTokenUnderCursor('/clear',6)).toBe('/clear');
 });
 it('init 提示词与 CLI 常量镜像一致',()=>{
  expect(INIT_PROMPT).toContain('AGENTS.md');
  expect(INIT_PROMPT).toContain('项目概述');
  expect(INIT_PROMPT).toContain('协作约定');
 });
 it('goal 提示词与 CLI 常量镜像一致',()=>{


  expect(GOAL_PROMPT).toContain('goal_write');
  expect(GOAL_PROMPT).toContain('未经我确认前不要修改任何文件');
 });
});

describe('expandSlashPrompt',()=>{
 it('/plan 不作为预置提示词展开',()=>{
  const expanded=expandSlashPrompt('/plan 为导出功能制定计划');
  expect(expanded).toBeNull();
 });
 it('带参数的 /goal 展开为预置提示词并拼接目标，参数保留多行',()=>{
  const expanded=expandSlashPrompt('/goal  完成导出功能\n并通过全部测试 ');
  expect(expanded).toEqual({kind:'prompt',text:`${GOAL_PROMPT}\n\n目标：完成导出功能\n并通过全部测试`});
 });
 it('无参数时返回用法提示',()=>{
  expect(expandSlashPrompt('/plan')).toBeNull();
  expect(expandSlashPrompt('/plan  ')).toBeNull();
  expect(expandSlashPrompt('/goal')).toEqual({kind:'usage',name:'goal'});
 });
 it('其余输入原样放行',()=>{
  expect(expandSlashPrompt('普通消息')).toBeNull();
  expect(expandSlashPrompt('/clear 帮我重开')).toBeNull();
  expect(expandSlashPrompt('/plans 多了字母')).toBeNull();
  expect(expandSlashPrompt('请执行 /plan 任务')).toBeNull();
 });
});
