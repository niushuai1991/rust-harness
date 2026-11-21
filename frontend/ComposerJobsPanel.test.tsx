import {cleanup,fireEvent,render,screen,waitFor} from '@testing-library/react';
import {afterEach,beforeEach,describe,expect,it,vi} from 'vitest';
import {ComposerJobsPanel} from './ComposerJobsPanel';
import type {JobInfo} from './types';

const {command}=vi.hoisted(()=>({command:vi.fn()}));
vi.mock('./api',()=>({command,errorText:(e:unknown)=>String(e)}));

const nowIso=()=>new Date().toISOString();
const running=(over:Partial<JobInfo>={}):JobInfo=>({id:'j1',command:'npm test',status:'running',cancelRequested:false,output:null,error:null,createdAt:nowIso(),...over});

beforeEach(()=>{command.mockReset()});
afterEach(cleanup);

describe('ComposerJobsPanel', () => {
 it('只显示运行中任务并展示时长', async () => {
  command.mockImplementation(async(name:string)=>name==='job_list'?[running(),running({id:'j2',command:'已完成的任务',status:'completed'})]:[]);
  render(<ComposerJobsPanel sessionId="s1" onClose={()=>{}}/>);
  await waitFor(()=>expect(command).toHaveBeenCalledWith('job_list',{sessionId:'s1'}));
  expect(await screen.findByTitle('npm test')).toBeInTheDocument();
  expect(screen.queryByTitle('已完成的任务')).not.toBeInTheDocument();
  expect(screen.getByLabelText('已运行时长').textContent).toMatch(/^\d{2,}:\d{2}$/);
 });

 it('取消按钮调用 job_cancel 后刷新', async () => {
  command.mockImplementation(async(name:string)=>{
   if(name==='job_list')return [running()];
   if(name==='job_cancel')return {};
   return [];
  });
  render(<ComposerJobsPanel sessionId="s1" onClose={()=>{}}/>);
  const button=await screen.findByRole('button',{name:'取消'});
  fireEvent.click(button);
  await waitFor(()=>expect(command).toHaveBeenCalledWith('job_cancel',{sessionId:'s1',jobId:'j1'}));
  await waitFor(()=>expect(command.mock.calls.filter(c=>c[0]==='job_list').length).toBeGreaterThan(1));
 });

 it('已请求取消的任务不再显示取消按钮', async () => {
  command.mockResolvedValue([running({cancelRequested:true})]);
  render(<ComposerJobsPanel sessionId="s1" onClose={()=>{}}/>);
  expect(await screen.findByText('已请求取消')).toBeInTheDocument();
  expect(screen.queryByRole('button',{name:'取消'})).not.toBeInTheDocument();
 });

 it('无运行中任务时显示空态', async () => {
  command.mockResolvedValue([running({status:'completed'})]);
  render(<ComposerJobsPanel sessionId="s1" onClose={()=>{}}/>);
  expect(await screen.findByText('本会话暂无运行中的后台任务。')).toBeInTheDocument();
 });

 it('点外部与 Escape 收起，点面板内部与触发按钮不收起', async () => {
  command.mockResolvedValue([]);
  const onClose=vi.fn();
  // 触发按钮（aria-controls 指向本面板）由 App 的 toggle 负责收起，组件豁免它。
  render(<div><button aria-controls="jobs-popover" type="button">触发</button><span>无关外部</span><ComposerJobsPanel sessionId="s1" onClose={onClose}/></div>);
  const panel=await screen.findByRole('region',{name:'运行中后台任务'});
  fireEvent.mouseDown(panel);
  fireEvent.mouseDown(screen.getByText('触发'));
  expect(onClose).not.toHaveBeenCalled();
  fireEvent.mouseDown(screen.getByText('无关外部'));
  expect(onClose).toHaveBeenCalledTimes(1);
  fireEvent.keyDown(document,{key:'Escape'});
  expect(onClose).toHaveBeenCalledTimes(2);
 });
});
