import {describe,expect,it} from 'vitest';
import {INIT_PROMPT,SLASH_COMMANDS,commandTokenUnderCursor,filterSlashCommands} from './slashCommands';

describe('斜杠命令注册表',()=>{
 it('包含桌面端可执行子集且无重复',()=>{
  const names=SLASH_COMMANDS.map(spec=>spec.name);
  expect(names).toEqual(['clear','compact','init','model','diff','status','help']);
  expect(new Set(names).size).toBe(names.length);
 });
 it('过滤：exact 优先于 prefix，按注册顺序，容忍前导斜杠',()=>{
  expect(filterSlashCommands('model')).toEqual([SLASH_COMMANDS[3]]);
  expect(filterSlashCommands('mo')).toEqual([SLASH_COMMANDS[3]]);
  expect(filterSlashCommands('/cl')).toEqual([SLASH_COMMANDS[0]]);
  expect(filterSlashCommands('s')).toEqual([SLASH_COMMANDS[5]]);
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
});
