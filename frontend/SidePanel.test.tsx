import {afterEach,beforeEach,describe,expect,it,vi} from 'vitest';
import {act,cleanup,fireEvent,render,screen} from '@testing-library/react';
import {SidePanel} from './SidePanel';
import {useSidePanel} from './sidePanelStore';
vi.mock('./ChangesView',()=>({ChangesView:({workspaceId}:{workspaceId:string})=><div>更改视图 {workspaceId}</div>}));
vi.mock('./PreviewView',()=>({PreviewView:({path}:{path:string|null})=><div>预览视图 {path??'空'}</div>}));
beforeEach(()=>{
 act(()=>{useSidePanel.getState().reset()});
 Element.prototype.setPointerCapture=Element.prototype.setPointerCapture??(()=>{});
 Element.prototype.releasePointerCapture=Element.prototype.releasePointerCapture??(()=>{});
 // jsdom 可能缺 PointerEvent：fireEvent.pointerMove 需要它携带 clientX。
 if(typeof window.PointerEvent!=='function'){
  const PE=class extends MouseEvent{pointerId:number;constructor(type:string,init:Record<string,unknown>={}){super(type,init as MouseEventInit);this.pointerId=Number(init.pointerId??0)}};
  (window as unknown as {PointerEvent:unknown}).PointerEvent=PE;
  (globalThis as unknown as {PointerEvent:unknown}).PointerEvent=PE;
 }
});
afterEach(cleanup);

describe('右侧面板容器',()=>{
 it('默认不渲染，openChanges 后显示更改 tab',()=>{
  render(<SidePanel workspaceId="w"/>);
  expect(screen.queryByRole('complementary',{name:'文件面板'})).not.toBeInTheDocument();
  act(()=>{useSidePanel.getState().openChanges()});
  const panel=screen.getByRole('complementary',{name:'文件面板'});
  expect(panel).toBeInTheDocument();
  expect(panel).toHaveTextContent('更改视图 w');
 });
 it('tab 切换更改与预览，预览携带当前文件路径',()=>{
  render(<SidePanel workspaceId="w"/>);
  act(()=>{useSidePanel.getState().openPreview('src/app.ts')});
  expect(screen.getByText('预览视图 src/app.ts')).toBeInTheDocument();
  fireEvent.click(screen.getByRole('tab',{name:'更改'}));
  expect(screen.getByText('更改视图 w')).toBeInTheDocument();
  fireEvent.click(screen.getByRole('tab',{name:'预览'}));
  expect(screen.getByText('预览视图 src/app.ts')).toBeInTheDocument();
 });
 it('Escape 关闭面板且不丢视图状态，重开仍在原视图',()=>{
  render(<SidePanel workspaceId="w"/>);
  act(()=>{useSidePanel.getState().openPreview('a.ts')});
  fireEvent.keyDown(document,{key:'Escape'});
  expect(screen.queryByRole('complementary',{name:'文件面板'})).not.toBeInTheDocument();
  act(()=>{useSidePanel.getState().openChanges()});
  expect(screen.getByText('更改视图 w')).toBeInTheDocument();
 });
 it('关闭按钮关闭面板',()=>{
  render(<SidePanel workspaceId="w"/>);
  act(()=>{useSidePanel.getState().openChanges()});
  fireEvent.click(screen.getByRole('button',{name:'关闭面板'}));
  expect(screen.queryByRole('complementary',{name:'文件面板'})).not.toBeInTheDocument();
 });
 it('拖拽左缘分隔条调宽并记忆，宽度限制在 280px 到 60% 之间',()=>{
  render(<SidePanel workspaceId="w"/>);
  act(()=>{useSidePanel.getState().openChanges()});
  const panel=screen.getByRole('complementary',{name:'文件面板'});
  const resizer=screen.getByRole('separator',{name:'调整面板宽度'});
  fireEvent.pointerDown(resizer,{pointerId:1});
  fireEvent.pointerMove(resizer,{pointerId:1,clientX:500});
  const innerWidth=window.innerWidth;
  expect(panel.style.width).toBe(`${Math.min(innerWidth-500,Math.round(innerWidth*0.6))}px`);
  fireEvent.pointerMove(resizer,{pointerId:1,clientX:innerWidth-100});
  expect(panel.style.width).toBe('280px');
  fireEvent.pointerUp(resizer,{pointerId:1});
  expect(localStorage.getItem('side-panel-width')).toBe('280');
  localStorage.removeItem('side-panel-width');
 });
 it('已保存的宽度在重开时恢复',()=>{
  localStorage.setItem('side-panel-width','520');
  render(<SidePanel workspaceId="w"/>);
  act(()=>{useSidePanel.getState().openChanges()});
  expect(screen.getByRole('complementary',{name:'文件面板'}).style.width).toBe('520px');
  localStorage.removeItem('side-panel-width');
 });
});
it('网页面板显示当前地址并可切换回文件预览',()=>{
 render(<SidePanel workspaceId="w"/>);
 act(()=>useSidePanel.getState().openWeb('https://example.com/'));
 expect(screen.getByRole('tab',{name:'网页'})).toHaveAttribute('aria-selected','true');
 expect(screen.getByText('https://example.com/')).toBeInTheDocument();
 fireEvent.click(screen.getByRole('tab',{name:'预览'}));
 expect(screen.getByText('预览视图 空')).toBeInTheDocument();
});
