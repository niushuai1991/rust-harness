import {afterEach,beforeEach,describe,expect,it,vi} from 'vitest';
import {cleanup,fireEvent,render,screen} from '@testing-library/react';
import {PlansPanel} from './PlansPanel';
const mocks=vi.hoisted(()=>({command:vi.fn()}));
vi.mock('./api',()=>({desktop:()=>true,command:mocks.command,errorText:(e:unknown)=>e instanceof Error?e.message:String(e)}));
beforeEach(()=>{mocks.command.mockReset()});
afterEach(cleanup);
const entry=(over:Partial<Record<string,unknown>>={})=>({sessionId:'s1',sessionTitle:'会话一',workspaceId:'w1',kind:'plan.state',data:{content:'最新计划'},updatedAt:'2026-11-06T10:00:00Z',...over} as never);

describe('计划与目标面板',()=>{
 it('按会话分组展示最新状态并渲染 todo 完成态',async()=>{
  mocks.command.mockResolvedValue([
   entry({kind:'todo.state',data:{items:[{text:'已完成项',done:true},{text:'进行中项'}]}}),
   entry({kind:'goal.state',data:{content:'本周目标'}}),
  ]);
  const {container}=render(<PlansPanel currentSessionId="s1" onOpenSession={vi.fn()} onClose={vi.fn()}/>);
  expect(await screen.findByText('会话一')).toBeInTheDocument();
  expect(container.querySelector('.todo-list li.done')?.textContent).toContain('已完成项');
  expect(container.querySelector('.todo-list li:not(.done)')?.textContent).toContain('进行中项');
  expect(screen.getByText('本周目标')).toBeInTheDocument();
 });
 it('查看会话回调携带会话与工作区标识',async()=>{
  mocks.command.mockResolvedValue([entry({sessionTitle:'跨工作区会话',workspaceId:'w2'})]);
  const onOpenSession=vi.fn();
  render(<PlansPanel currentSessionId="s9" onOpenSession={onOpenSession} onClose={vi.fn()}/>);
  await screen.findByText('跨工作区会话');
  fireEvent.click(screen.getByRole('button',{name:'查看会话'}));
  expect(onOpenSession).toHaveBeenCalledWith('s1','w2');
 });
 it('演变历史懒加载并只保留状态事件',async()=>{
  mocks.command.mockImplementation(async(name:string,args:any)=>{
   if(name==='plan_overview')return [entry()];
   if(name==='session_events'&&args.sessionId==='s1')return [
    {id:'e1',sessionId:'s1',seq:'1',kind:'user.message',data:{content:'你好'},createdAt:'2026-11-06T09:00:00Z'},
    {id:'e2',sessionId:'s1',seq:'2',kind:'plan.state',data:{content:'旧计划'},createdAt:'2026-11-06T09:05:00Z'},
    {id:'e3',sessionId:'s1',seq:'3',kind:'plan.state',data:{content:'最新计划'},createdAt:'2026-11-06T10:00:00Z'},
   ];
   return [];
  });
  render(<PlansPanel currentSessionId="s1" onOpenSession={vi.fn()} onClose={vi.fn()}/>);
  await screen.findByText('会话一');
  expect(screen.queryByText('旧计划')).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole('button',{name:'演变历史'}));
  await screen.findByText('旧计划');
  expect(screen.queryByText('你好')).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole('button',{name:'收起历史'}));
  expect(screen.queryByText('旧计划')).not.toBeInTheDocument();
 });
 it('无任何状态时显示空态',async()=>{
  mocks.command.mockResolvedValue([]);
  render(<PlansPanel currentSessionId="" onOpenSession={vi.fn()} onClose={vi.fn()}/>);
  expect(await screen.findByText('还没有任何计划或目标')).toBeInTheDocument();
 });
 it('加载失败显示错误且不假装成功',async()=>{
  mocks.command.mockRejectedValue(new Error('查询失败'));
  render(<PlansPanel currentSessionId="" onOpenSession={vi.fn()} onClose={vi.fn()}/>);
  expect(await screen.findByText('查询失败')).toBeInTheDocument();
 });
});
