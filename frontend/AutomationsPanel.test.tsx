import {afterEach,beforeEach,expect,it,vi} from 'vitest';
import {cleanup,fireEvent,render,screen,waitFor} from '@testing-library/react';
import {AutomationsPanel} from './AutomationsPanel';
import type {AutomationInfo,SessionInfo} from './types';
const mocks=vi.hoisted(()=>({command:vi.fn()}));
vi.mock('./api',()=>({desktop:()=>true,command:mocks.command,errorText:(e:unknown)=>e instanceof Error?e.message:String(e)}));
beforeEach(()=>{mocks.command.mockReset();mocks.command.mockResolvedValue([])});
afterEach(cleanup);
const sessions:SessionInfo[]=[{id:'s1',workspaceId:'w1',title:'重构讨论',createdAt:'',updatedAt:''},{id:'s2',workspaceId:'w1',title:'发布准备',createdAt:'',updatedAt:''}];
const daily:AutomationInfo={id:'a1',title:'每日代码评审',prompt:'请检查提交',sessionId:'s1',workspaceId:'w1',kind:'daily',at:null,time:'09:30',weekdays:null,monthDay:null,month:null,enabled:true,status:'active',nextRunAt:'2026-11-08T01:30:00Z',lastRunAt:null,runCount:12,runs:[{id:'r1',scheduledAt:null,triggeredAt:'2026-11-07T01:30:00Z',trigger:'schedule',status:'succeeded',sessionId:'s1',error:null}],createdAt:'',updatedAt:''};
const paused:AutomationInfo={...daily,id:'a2',title:'发布周报',sessionId:null,kind:'weekly',weekdays:[5],time:'18:00',enabled:false,status:'paused',nextRunAt:null,runCount:0,runs:[]};
const renderPanel=()=>render(<AutomationsPanel workspaceId="w1" sessions={sessions}/>);

it('列表渲染周期描述、目标会话与运行历史计数',async()=>{
  mocks.command.mockResolvedValue([daily,paused]);
  renderPanel();
  expect(await screen.findByText(/每天 09:30 · 会话「重构讨论」 · 已执行 12 次/)).toBeInTheDocument();
  expect(screen.getByText(/每周五 18:00 · 每次新建会话 · 已执行 0 次/)).toBeInTheDocument();
  expect(screen.getByText(/运行历史（最近 1 次）/)).toBeInTheDocument();
  expect(screen.getByText('暂无执行记录。')).toBeInTheDocument();
});

it('新建表单默认每天 09:30，切换单次后默认时间秒归零并提交完整载荷',async()=>{
  mocks.command.mockResolvedValue([daily]);
  renderPanel();
  await screen.findByText(/每天 09:30/);
  fireEvent.click(screen.getByRole('button',{name:/新建任务/}));
  expect(screen.getByLabelText('时间（24 小时制）')).toHaveValue('09:30');
  const kind=screen.getByLabelText(/执行周期/);
  fireEvent.change(kind,{target:{value:'once'}});
  const onceInput=screen.getByLabelText(/执行时间（含秒，24 小时制）/);
  // jsdom 会剥离 :00 秒位；真实 Chromium 在 step=1 下保留，正则两者兼容。
  expect((onceInput as HTMLInputElement).value).toMatch(/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(:00)?$/);
  // 过去时间提交被拦截且保留输入
  fireEvent.change(onceInput,{target:{value:'2020-01-01T08:00:00'}});
  fireEvent.change(screen.getByLabelText(/任务内容/),{target:{value:'到期要做的事'}});
  fireEvent.click(screen.getByRole('button',{name:'创建任务'}));
  expect(await screen.findByText('执行时间必须在未来。')).toBeInTheDocument();
  expect(mocks.command).not.toHaveBeenCalledWith('automation_create',expect.anything());
  // 未来时间提交，payload 为 ISO 字符串
  fireEvent.change(onceInput,{target:{value:'2099-01-01T08:00:00'}});
  fireEvent.click(screen.getByRole('button',{name:'创建任务'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('automation_create',expect.objectContaining({kind:'once'})));
  const args=mocks.command.mock.calls.find(c=>c[0]==='automation_create')![1];
  expect(args.prompt).toBe('到期要做的事');
  expect(args.sessionId).toBeNull();
  expect(args.at).toBe(new Date('2099-01-01T08:00:00').toISOString());
  expect(args.time).toBeNull();
});

it('每周模式展示周几多选，未选提交被拦截',async()=>{
  mocks.command.mockResolvedValue([daily]);
  renderPanel();
  await screen.findByText(/每天 09:30/);
  fireEvent.click(screen.getByRole('button',{name:/新建任务/}));
  fireEvent.change(screen.getByLabelText(/执行周期/),{target:{value:'weekly'}});
  expect(screen.getByLabelText('时间（24 小时制）')).toBeInTheDocument();
  expect(screen.getByText('周几')).toBeInTheDocument();
  fireEvent.change(screen.getByLabelText(/任务内容/),{target:{value:'周报'}});
  fireEvent.click(screen.getByRole('button',{name:'创建任务'}));
  expect(screen.getByText('请至少选择一个周几。')).toBeInTheDocument();
  fireEvent.click(screen.getByLabelText('一'));
  fireEvent.click(screen.getByLabelText('五'));
  fireEvent.click(screen.getByRole('button',{name:'创建任务'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('automation_create',expect.objectContaining({kind:'weekly',weekdays:[1,5],time:'09:30'})));
});

it('删除走二段确认，启停调用 set_enabled',async()=>{
  mocks.command.mockResolvedValue([daily]);
  renderPanel();
  await screen.findByText(/每日代码评审/);
  fireEvent.click(screen.getByRole('button',{name:/删除/}));
  expect(screen.getByText(/确定删除该自动化任务/)).toBeInTheDocument();
  fireEvent.click(screen.getByRole('button',{name:'取消'}));
  expect(screen.queryByText(/确定删除该自动化任务/)).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole('button',{name:/删除/}));
  fireEvent.click(screen.getByRole('button',{name:'确认删除'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('automation_delete',{id:'a1'}));
  fireEvent.click(screen.getByRole('checkbox'));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('automation_set_enabled',{id:'a1',enabled:false}));
});

it('编辑回填草稿并提交 automation_update',async()=>{
  mocks.command.mockResolvedValue([daily]);
  renderPanel();
  await screen.findByText(/每日代码评审/);
  fireEvent.click(screen.getByRole('button',{name:/编辑/}));
  expect(screen.getByLabelText(/标题/)).toHaveValue('每日代码评审');
  expect(screen.getByLabelText(/目标会话/)).toHaveValue('s1');
  expect(screen.getByLabelText('时间（24 小时制）')).toHaveValue('09:30');
  fireEvent.change(screen.getByLabelText('时间（24 小时制）'),{target:{value:'22:00'}});
  fireEvent.click(screen.getByRole('button',{name:'保存修改'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('automation_update',expect.objectContaining({id:'a1'})));
  const args=mocks.command.mock.calls.find(c=>c[0]==='automation_update')![1];
  expect(args.params).toEqual(expect.objectContaining({kind:'daily',time:'22:00',sessionId:'s1',workspaceId:'w1'}));
});

it('加载失败显示错误',async()=>{
  mocks.command.mockRejectedValue(new Error('查询失败'));
  renderPanel();
  expect(await screen.findByText('查询失败')).toBeInTheDocument();
});
