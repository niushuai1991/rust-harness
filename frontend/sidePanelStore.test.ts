import {describe,expect,it} from 'vitest';
import {useSidePanel} from './sidePanelStore';

describe('右侧面板状态',()=>{
 it('openPreview 打开面板、切到预览并记录文件路径',()=>{
  useSidePanel.getState().reset();
  useSidePanel.getState().openPreview('src/app.ts');
  const state=useSidePanel.getState();
  expect(state.open).toBe(true);
  expect(state.view).toBe('preview');
  expect(state.previewPath).toBe('src/app.ts');
 });
 it('openChanges 打开面板并回到更改视图，保留预览路径',()=>{
  useSidePanel.getState().reset();
  useSidePanel.getState().openPreview('a.ts');
  useSidePanel.getState().openChanges();
  const state=useSidePanel.getState();
  expect(state.open).toBe(true);
  expect(state.view).toBe('changes');
  expect(state.previewPath).toBe('a.ts');
 });
 it('toggle 切换开合，close 只隐藏不丢视图状态，reset 全部还原',()=>{
  useSidePanel.getState().reset();
  useSidePanel.getState().openPreview('a.ts');
  useSidePanel.getState().toggle();
  expect(useSidePanel.getState().open).toBe(false);
  useSidePanel.getState().toggle();
  expect(useSidePanel.getState().open).toBe(true);
  useSidePanel.getState().close();
  expect(useSidePanel.getState().previewPath).toBe('a.ts');
  useSidePanel.getState().reset();
  const state=useSidePanel.getState();
  expect(state.open).toBe(false);
  expect(state.view).toBe('changes');
  expect(state.previewPath).toBeNull();
 });
});
