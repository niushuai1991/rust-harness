import {afterEach,beforeEach,describe,expect,it,vi} from 'vitest';
import {act,cleanup,fireEvent,render,screen,waitFor,within} from '@testing-library/react';
import {ChangesView} from './ChangesView';
const mocks=vi.hoisted(()=>({command:vi.fn()}));
vi.mock('./api',()=>({desktop:()=>true,command:mocks.command,errorText:(e:unknown)=>e instanceof Error?e.message:String(e)}));
beforeEach(()=>{mocks.command.mockReset()});
afterEach(()=>{cleanup();vi.useRealTimers()});
const change=(over:Partial<Record<string,unknown>>={})=>({path:'a.rs',kind:'modified',conflict:false,beforeHash:'before',afterHash:'after',sourceHash:null,beforeText:'old',afterText:'new',...over} as never);
const UNIFIED='--- a/a.rs\n+++ b/a.rs\n@@ -1,2 +1,2 @@\n-old line\n+new line\n ctx\n';
function mockBackend(list:unknown,diff:unknown={path:'a.rs',unified:UNIFIED}){
 mocks.command.mockImplementation(async(name:string)=>{
  if(name==='workspace_diff')return list;
  if(name==='workspace_file_diff')return diff;
  return [];
 });
}

describe('改动审阅视图（右侧面板）',()=>{
 it('渲染文件列表与冲突标注，展开懒加载行级 diff 并显示增删统计',async()=>{
  mockBackend([change(),change({path:'conflict.rs',conflict:true})]);
  render(<ChangesView workspaceId="w"/>);
  await screen.findByText('a.rs');
  expect(screen.getByText('冲突')).toBeInTheDocument();
  expect(screen.getByLabelText('选择 conflict.rs')).toBeDisabled();
  expect(screen.queryByText('-old line')).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole('button',{name:/a\.rs$/}));
  await screen.findByText('+new line');
  expect(screen.getByText('-old line').closest('.diff-line')).toHaveClass('del');
  expect(screen.getByText('+1')).toHaveClass('stat-add');
  expect(screen.getByText('−1')).toHaveClass('stat-del');
 });
 it('conflict 文件展开时显示处理提示，二进制占位不渲染 diff',async()=>{
  mockBackend([change({path:'conflict.rs',conflict:true})],{path:'conflict.rs',unified:null});
  render(<ChangesView workspaceId="w"/>);
  await screen.findByText('conflict.rs');
  fireEvent.click(screen.getByRole('button',{name:/conflict\.rs$/}));
  await screen.findByText(/原项目在同步前也发生了变更/);
  await screen.findByText(/二进制文件或文本超过 1 MiB/);
 });
 it('只应用选中文件，传递完整审阅快照且需要二次确认',async()=>{
  const reviewed=change() as never;
  mockBackend([reviewed,change({path:'b.rs'}),change({path:'conflict.rs',conflict:true})]);
  render(<ChangesView workspaceId="w"/>);
  await screen.findByText('a.rs');
  fireEvent.click(screen.getByLabelText('选择 b.rs'));
  expect(screen.getByLabelText('选择 conflict.rs')).toBeDisabled();
  fireEvent.click(screen.getByRole('button',{name:'同步选中文件'}));
  expect(mocks.command).not.toHaveBeenCalledWith('workspace_apply',expect.anything());
  fireEvent.click(screen.getByRole('button',{name:'确认应用更改'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('workspace_apply',{workspaceId:'w',paths:['a.rs'],reviewed:[reviewed]}));
 });
 it('宿主拒绝过期审阅时保留错误且不声称已同步',async()=>{
  mockBackend([change()]);
  mocks.command.mockImplementation(async(name:string)=>{
   if(name==='workspace_diff')return [change()];
   if(name==='workspace_file_diff')return {path:'a.rs',unified:UNIFIED};
   if(name==='workspace_apply')throw new Error('审阅后副本已改变，请重新审阅');
   return [];
  });
  render(<ChangesView workspaceId="w"/>);
  await screen.findByText('a.rs');
  fireEvent.click(screen.getByRole('button',{name:/a\.rs$/}));
  await screen.findByText('+new line');
  fireEvent.click(screen.getByRole('button',{name:'同步选中文件'}));
  fireEvent.click(screen.getByRole('button',{name:'确认应用更改'}));
  await screen.findByText('审阅后副本已改变，请重新审阅');
  expect(screen.queryByText('已将选中更改同步到原项目。')).not.toBeInTheDocument();
  expect(screen.getByText('+new line')).toBeInTheDocument();
 });
 it('刷新差异后需要重新确认，保持展开的文件自动重算差异',async()=>{
  mockBackend([change()]);
  render(<ChangesView workspaceId="w"/>);
  await screen.findByText('a.rs');
  fireEvent.click(screen.getByRole('button',{name:/a\.rs$/}));
  await screen.findByText('+new line');
  fireEvent.click(screen.getByRole('button',{name:'同步选中文件'}));
  expect(screen.getByRole('button',{name:'确认应用更改'})).toBeInTheDocument();
  mockBackend([change({afterHash:'updated',afterText:'updated text'})],{path:'a.rs',unified:'--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,1 @@\n+updated text\n'});
  fireEvent.click(screen.getByRole('button',{name:'刷新'}));
  await waitFor(()=>expect(screen.queryByRole('button',{name:'确认应用更改'})).not.toBeInTheDocument());
  await screen.findByText('+updated text');
  expect(screen.queryByText('+new line')).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole('button',{name:'同步选中文件'}));
  fireEvent.click(screen.getByRole('button',{name:'确认应用更改'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('workspace_apply',{workspaceId:'w',paths:['a.rs'],reviewed:[change({afterHash:'updated',afterText:'updated text'})]}));
 });
 it('轮询刷新保留用户勾选，消失文件移出选择',async()=>{
  vi.useFakeTimers();
  mockBackend([change(),change({path:'b.rs'})]);
  render(<ChangesView workspaceId="w"/>);
  await act(async()=>{await vi.advanceTimersByTimeAsync(0)});
  expect(screen.getByLabelText('选择 b.rs')).toBeChecked();
  fireEvent.click(screen.getByLabelText('选择 b.rs'));
  mockBackend([change()]);
  await act(async()=>{await vi.advanceTimersByTimeAsync(5000)});
  expect(screen.queryByLabelText('选择 b.rs')).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole('button',{name:'同步选中文件'}));
  fireEvent.click(screen.getByRole('button',{name:'确认应用更改'}));
  await act(async()=>{await vi.advanceTimersByTimeAsync(0)});
  expect(mocks.command).toHaveBeenCalledWith('workspace_apply',{workspaceId:'w',paths:['a.rs'],reviewed:[change()]});
 });
 it('空列表显示空态，失败显示错误',async()=>{
  mockBackend([]);
  render(<ChangesView workspaceId="w"/>);
  await screen.findByText('工作副本没有待同步更改');
  mockBackend([]);
  mocks.command.mockRejectedValue(new Error('查询失败'));
  fireEvent.click(screen.getByRole('button',{name:'刷新'}));
  await screen.findByText('查询失败');
 });
});
