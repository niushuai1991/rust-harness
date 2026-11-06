import {afterEach,beforeEach,describe,expect,it,vi} from 'vitest';
import {cleanup,fireEvent,render,screen,waitFor} from '@testing-library/react';
import {BackgroundTasksPanel} from './BackgroundTasksPanel';
const {command}=vi.hoisted(()=>({command:vi.fn()}));
vi.mock('./api',()=>({desktop:()=>true,command,errorText:(e:unknown)=>e instanceof Error?e.message:String(e)}));
beforeEach(()=>{command.mockReset()});
afterEach(cleanup);
const job={id:'job-1',command:'cargo test',status:'running',cancelRequested:false,output:null,error:null,createdAt:'2026-11-06T01:00:00Z'};
const schedule={id:'sch-1',sessionId:'a',workspaceId:'w',at:'2099-01-01T00:00:00Z',task:'以后执行',modelId:'missing',status:'pending'};
describe('后台任务面板',()=>{
 it('加载后台任务与定时任务列表，允许取消运行中的后台任务',async()=>{
  command.mockImplementation(async name=>{
   if(name==='job_list')return [job];
   if(name==='schedule_list')return [schedule];
   if(name==='job_cancel')return;
   throw new Error('未知命令');
  });
  render(<BackgroundTasksPanel sessionId="a" onClose={()=>{}}/>);
  await screen.findByText('cargo test');
  expect(screen.getByText('以后执行')).toBeInTheDocument();
  expect(screen.getByText('等待执行')).toBeInTheDocument();
  fireEvent.click(screen.getByRole('button',{name:'取消任务'}));
  await waitFor(()=>expect(command).toHaveBeenCalledWith('job_cancel',{sessionId:'a',jobId:'job-1'}));
  await waitFor(()=>expect(command.mock.calls.filter(([name])=>name==='job_list')).toHaveLength(2));
 });
 it('终态任务不提供取消，作用域错误可见',async()=>{
  command.mockImplementation(async name=>{
   if(name==='job_list')return [job];
   if(name==='schedule_list')return [];
   if(name==='job_cancel')throw new Error('任务不存在或不属于本会话');
   throw new Error('未知命令');
  });
  const {unmount}=render(<BackgroundTasksPanel sessionId="a" onClose={()=>{}}/>);
  await screen.findByText('cargo test');
  fireEvent.click(screen.getByRole('button',{name:'取消任务'}));
  await screen.findByText('任务不存在或不属于本会话');
  unmount();
  command.mockImplementation(async name=>{
   if(name==='job_list')return [{...job,status:'cancelled',cancelRequested:true}];
   if(name==='schedule_list')return [];
   throw new Error('未知命令');
  });
  render(<BackgroundTasksPanel sessionId="a" onClose={()=>{}}/>);
  await screen.findByText('cargo test');
  expect(screen.queryByRole('button',{name:'取消任务'})).not.toBeInTheDocument();
 });
 it('删除定时任务需要确认并调用宿主',async()=>{
  command.mockImplementation(async name=>{
   if(name==='job_list')return [];
   if(name==='schedule_list')return [schedule];
   throw new Error('未知命令');
  });
  render(<BackgroundTasksPanel sessionId="a" onClose={()=>{}}/>);
  await screen.findByText('以后执行');
  fireEvent.click(screen.getByRole('button',{name:'删除任务'}));
  expect(command).not.toHaveBeenCalledWith('schedule_delete',expect.anything());
  fireEvent.click(screen.getByRole('button',{name:'确认删除'}));
  await waitFor(()=>expect(command).toHaveBeenCalledWith('schedule_delete',{sessionId:'a',id:'sch-1'}));
 });
 it('会话切换后重新加载',async()=>{
  command.mockImplementation(async name=>{
   if(name==='job_list')return [];
   if(name==='schedule_list')return [];
   throw new Error('未知命令');
  });
  const {rerender}=render(<BackgroundTasksPanel sessionId="a" onClose={()=>{}}/>);
  await waitFor(()=>expect(command).toHaveBeenCalledWith('job_list',{sessionId:'a'}));
  rerender(<BackgroundTasksPanel sessionId="b" onClose={()=>{}}/>);
  await waitFor(()=>expect(command).toHaveBeenCalledWith('job_list',{sessionId:'b'}));
 });
});
