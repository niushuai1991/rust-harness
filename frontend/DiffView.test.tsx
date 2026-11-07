import {describe,expect,it} from 'vitest';
import {parseUnifiedDiff,diffStats,DiffView} from './DiffView';
import {render,screen} from '@testing-library/react';

const MODIFIED=['--- a/src/app.ts','+++ b/src/app.ts','@@ -1,3 +1,3 @@',' const a=1;','-const b=2;','+const b=3;',' const c=4;','','@@ -10,2 +10,2 @@','-old()','+new()'].join('\n');

describe('parseUnifiedDiff',()=>{
 it('解析文件头、hunk 行号与增删改行',()=>{
  const lines=parseUnifiedDiff(MODIFIED);
  expect(lines[0]).toEqual({type:'hunk',oldNo:null,newNo:null,text:'@@ -1,3 +1,3 @@'});
  expect(lines[1]).toEqual({type:'ctx',oldNo:1,newNo:1,text:'const a=1;'});
  expect(lines[2]).toEqual({type:'del',oldNo:2,newNo:null,text:'const b=2;'});
  expect(lines[3]).toEqual({type:'add',oldNo:null,newNo:2,text:'const b=3;'});
  expect(lines[4]).toEqual({type:'ctx',oldNo:3,newNo:3,text:'const c=4;'});
 });
 it('hunk 之间重置行号',()=>{
  const lines=parseUnifiedDiff(MODIFIED);
  const second=lines.at(-2)!;
  expect(second.type).toBe('del');
  expect(second.oldNo).toBe(10);
  expect(lines.at(-1)!.newNo).toBe(10);
 });
 it('added 文件旧侧行号为空，deleted 新侧行号为空',()=>{
  const added=parseUnifiedDiff(['--- /dev/null','+++ b/new.ts','@@ -0,0 +1,2 @@','+a','+b'].join('\n'));
  expect(added[1]).toEqual({type:'add',oldNo:null,newNo:1,text:'a'});
  expect(added[2].newNo).toBe(2);
  const deleted=parseUnifiedDiff(['--- a/old.ts','+++ /dev/null','@@ -1,2 +0,0 @@','-a','-b'].join('\n'));
  expect(deleted[1]).toEqual({type:'del',oldNo:1,newNo:null,text:'a'});
 });
 it('上下文空行推进行号，末尾换行不产生幽灵行',()=>{
  const lines=parseUnifiedDiff('@@ -1,2 +1,2 @@\n a\n\n b\n');
  expect(lines.map(l=>l.type)).toEqual(['hunk','ctx','ctx','ctx']);
  expect(lines[2].text).toBe('');
  expect(lines[2].oldNo).toBe(2);
  expect(lines[3].oldNo).toBe(3);
 });
 it('hunk 之后以 --- 开头的删除行不再误判为文件头',()=>{
  const lines=parseUnifiedDiff('@@ -1,2 +1,2 @@\n---x\n+--y\n');
  expect(lines[1].type).toBe('del');
  expect(lines[1].text).toBe('--x');
 });
 it('截断提示行归为 note',()=>{
  const lines=parseUnifiedDiff('@@ -1,1 +1,1 @@\n a\n…（diff 超过 2000 行，已截断显示）\n');
  expect(lines.at(-1)!.type).toBe('note');
 });
 it('diffStats 统计增删行数',()=>{
  expect(diffStats(parseUnifiedDiff(MODIFIED))).toEqual({add:2,del:2});
 });
});

describe('DiffView',()=>{
 it('渲染行号列与着色行，hunk 行整行展示',()=>{
  render(<DiffView diff={MODIFIED}/>);
  expect(screen.getByText('@@ -1,3 +1,3 @@')).toHaveClass('diff-hunk');
  const del=screen.getByText('-const b=2;').closest('.diff-line');
  expect(del).toHaveClass('del');
  expect(del!.querySelector('.diff-no')?.textContent).toBe('2');
  expect(screen.getByText('+const b=3;').closest('.diff-line')).toHaveClass('add');
 });
 it('note 行渲染为截断提示',()=>{
  render(<DiffView diff={['@@ -1,1 +1,1 @@',' a','…（diff 超过 2000 行，已截断显示）'].join('\n')}/>);
  expect(screen.getByText(/已截断显示/)).toHaveClass('diff-note');
 });
});
