import {afterEach,beforeEach,describe,expect,it,vi} from 'vitest';
import {act,cleanup,fireEvent,render,screen,waitFor,within} from '@testing-library/react';
import App from './App';
import {ChangesView} from './ChangesView';
import {GOAL_PROMPT,PLAN_PROMPT} from './slashCommands';
import {useChat} from './store';
import {useSidePanel} from './sidePanelStore';
import {open,save} from '@tauri-apps/plugin-dialog';
import type {LiveEvent,SessionEvent} from './types';
const mocks=vi.hoisted(()=>({command:vi.fn(),send:vi.fn()}));
vi.mock('./api',()=>({desktop:()=>true,command:mocks.command,send:mocks.send,errorText:(e:unknown)=>e instanceof Error?e.message:String(e)}));
vi.mock('@tauri-apps/plugin-dialog',()=>({open:vi.fn(),save:vi.fn()}));
const event=(id:string,sessionId:string,seq:string,content:string):SessionEvent=>({id,sessionId,seq,kind:'assistant.message',data:{content},createdAt:''});
beforeEach(()=>{useChat.setState({events:{},streams:{},statuses:{}});useSidePanel.getState().reset();mocks.command.mockReset();mocks.send.mockReset();mocks.command.mockImplementation(async(name:string,args:any)=>{
 if(name==='workspace_list')return [{id:'w',sourcePath:'D:/project',replicaPath:'D:/copy',createdAt:''}];
 if(name==='model_list')return [{id:'m',name:'测试模型',compatibility:'standard',providerId:'custom',apiMode:'chatCompletions',maxOutputTokens:null,baseUrl:'https://example.test',model:'m',credentialRef:'',proxy:null,efforts:['low','high'],effort:null,enabled:true,builtin:false}];
 if(name==='provider_list')return [{id:'custom',name:'自定义供应商',builtin:false,compatibility:'standard',endpoints:[{apiMode:'chatCompletions',baseUrl:'https://example.test'}],defaultModel:'m',credentialRef:'',proxy:null,enabled:true,builtinModels:[],disabledModels:[]}];
 if(name==='default_selection_get')return null;
 if(name==='session_list')return [{id:'a',workspaceId:'w',title:'会话 A'},{id:'b',workspaceId:'w',title:'会话 B'}];
 if(name==='session_events')return [event(`e-${args.sessionId}`,args.sessionId,'1',`内容 ${args.sessionId}`)];
 if(name==='approval_list')return [];
 return [];
})});
afterEach(()=>{cleanup();vi.useRealTimers()});
describe('聊天交互',()=>{
 it('新建会话的迟到响应不能污染已切换工作区或改变发送目标',async()=>{
  let resolveCreation!:(value:unknown)=>void;
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>{
   if(name==='workspace_list')return [{id:'w',sourcePath:'D:/project-A'},{id:'other',sourcePath:'D:/project-B'}];
   if(name==='session_create')return new Promise(resolve=>{resolveCreation=resolve});
   if(name==='session_list'&&args.workspaceId==='other')return [{id:'other-session',workspaceId:'other',title:'B 的会话'}];
   return original(name,args);
  });
  render(<App/>);await screen.findByText('内容 a');
  fireEvent.click(screen.getByRole('button',{name:'新建会话'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('session_create',{workspaceId:'w',title:'新会话'}));
  fireEvent.change(screen.getByLabelText('工作区'),{target:{value:'other'}});
  await screen.findByText('内容 other-session');
  await act(async()=>resolveCreation({id:'late-session',workspaceId:'w',title:'A 迟到的新会话'}));
  expect(screen.queryByRole('button',{name:'A 迟到的新会话'})).not.toBeInTheDocument();
  expect(screen.getByText('内容 other-session')).toBeInTheDocument();
  fireEvent.change(screen.getByLabelText('消息'),{target:{value:'仅发送给 B'}});
  fireEvent.click(screen.getByRole('button',{name:'发送消息'}));
  await waitFor(()=>expect(mocks.send).toHaveBeenCalledWith('other-session','仅发送给 B','m',expect.any(Function),undefined,'high'));
 });
 it('切换会话只显示该会话的消息',async()=>{render(<App/>);await screen.findByText('内容 a');fireEvent.click(screen.getByRole('button',{name:/会话 B/}));await screen.findByText('内容 b');expect(screen.queryByText('内容 a')).not.toBeInTheDocument()});
 it('完成回合折叠过程但始终显示 Markdown 最终总结',async()=>{
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>name==='session_events'?[{id:'start',sessionId:'a',seq:'1',kind:'turn.start',data:{runId:'r'},createdAt:'2026-11-05T00:00:00Z'},{id:'user',sessionId:'a',seq:'2',kind:'user.message',data:{content:'实现预览'},createdAt:'2026-11-05T00:00:01Z'},{id:'process',sessionId:'a',seq:'3',kind:'assistant.message',data:{content:'正在检查。'},createdAt:'2026-11-05T00:00:10Z'},{id:'call',sessionId:'a',seq:'4',kind:'tool.call',data:{name:'shell',arguments:{command:'cargo test'}},createdAt:'2026-11-05T00:01:00Z'},{id:'result',sessionId:'a',seq:'5',kind:'tool.result',data:{name:'shell',content:'ok'},createdAt:'2026-11-05T00:02:00Z'},{id:'final',sessionId:'a',seq:'6',kind:'assistant.message',data:{content:'## 最终总结\n\n- 已完成 **Markdown** 预览'},createdAt:'2026-11-05T00:03:00Z'},{id:'end',sessionId:'a',seq:'7',kind:'turn.end',data:{runId:'r',status:'completed'},createdAt:'2026-11-05T00:03:22Z'}]:original(name,args));
  render(<App/>);
  await screen.findByRole('heading',{name:'最终总结',level:2});
  expect(screen.getByText('本轮已完成，用时3分22秒')).toBeVisible();
  expect(screen.getByText('正在检查。')).toBeVisible();
  fireEvent.click(screen.getByText('本轮已完成，用时3分22秒'));
  expect(screen.getByText('最终总结')).toBeVisible();
  expect(screen.getByText('正在检查。')).not.toBeVisible();
 });
 it('发送期间合并增量，结束后读取持久化事件',async()=>{let release!:()=>void;mocks.send.mockImplementation(async(_id:string,_text:string,_model:string,receive:(e:LiveEvent)=>void)=>{receive({type:'delta',sessionId:'a',requestId:'r',index:0,text:'正在输出',reasoning:false});await new Promise<void>(r=>release=r)});render(<App/>);await screen.findByText('内容 a');fireEvent.change(screen.getByLabelText('消息'),{target:{value:'你好'}});fireEvent.click(screen.getByRole('button',{name:'发送消息'}));await screen.findByText('正在输出');expect(screen.getByRole('button',{name:'取消生成'})).toBeInTheDocument();release();await waitFor(()=>expect(screen.queryByText('正在输出')).not.toBeInTheDocument());expect(mocks.send).toHaveBeenCalledWith('a','你好','m',expect.any(Function),undefined,'high')});
 it('发送错误可见且不插入假成功消息',async()=>{mocks.send.mockRejectedValue(new Error('连接失败'));render(<App/>);await screen.findByText('内容 a');fireEvent.change(screen.getByLabelText('消息'),{target:{value:'请求'}});fireEvent.click(screen.getByRole('button',{name:'发送消息'}));await screen.findByText('连接失败');expect(screen.getByRole('button',{name:'发送消息'})).toBeInTheDocument()});
 it('审批通过传递真实请求id',async()=>{const original=mocks.command.getMockImplementation()!;mocks.command.mockImplementation(async(name,args)=>name==='approval_list'?[{id:'approval-1',sessionId:'a',tool:'shell',arguments:{command:'cargo test'},reason:'执行测试'}]:original(name,args));render(<App/>);await screen.findByRole('dialog',{name:'操作需要你的批准'});fireEvent.click(screen.getByRole('button',{name:'批准此次操作'}));await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('approval_resolve',{id:'approval-1',allow:true}))});
});
describe('同步审阅（右侧面板）',()=>{
 const reviewed={path:'a.rs',kind:'modified',conflict:false,beforeHash:'before-a',afterHash:'reviewed-a',sourceHash:'source-a',beforeText:'old',afterText:'new'};
 const UNIFIED='--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,1 @@\n+new text\n';
 function reviewBackend(list:unknown[],applyError?:string,diff:unknown={path:'a.rs',unified:UNIFIED}){
  mocks.command.mockImplementation(async(name:string)=>{
   if(name==='workspace_diff')return list;
   if(name==='workspace_file_diff')return diff;
   if(name==='workspace_apply'){if(applyError)throw new Error(applyError);return []}
   return [];
  });
 }
 it('只应用选中文件，传递完整审阅快照且需要二次确认',async()=>{
  reviewBackend([reviewed,{...reviewed,path:'b.rs'},{...reviewed,path:'conflict.rs',conflict:true}]);
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
  reviewBackend([reviewed],'审阅后副本已改变，请重新审阅');
  render(<ChangesView workspaceId="w"/>);
  await screen.findByText('a.rs');
  fireEvent.click(screen.getByRole('button',{name:/a\.rs$/}));
  await screen.findByText('+new text');
  fireEvent.click(screen.getByRole('button',{name:'同步选中文件'}));
  fireEvent.click(screen.getByRole('button',{name:'确认应用更改'}));
  await screen.findByText('审阅后副本已改变，请重新审阅');
  expect(screen.queryByText('已将选中更改同步到原项目。')).not.toBeInTheDocument();
  expect(screen.getByText('+new text')).toBeInTheDocument();
 });
 it('刷新差异后需要重新确认，保持展开的文件自动重算差异',async()=>{
  reviewBackend([reviewed]);
  render(<ChangesView workspaceId="w"/>);
  await screen.findByText('a.rs');
  fireEvent.click(screen.getByRole('button',{name:/a\.rs$/}));
  await screen.findByText('+new text');
  fireEvent.click(screen.getByRole('button',{name:'同步选中文件'}));
  expect(screen.getByRole('button',{name:'确认应用更改'})).toBeInTheDocument();
  reviewBackend([{...reviewed,afterHash:'updated',afterText:'updated text'}],undefined,{path:'a.rs',unified:'--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,1 @@\n+updated text\n'});
  fireEvent.click(screen.getByRole('button',{name:'刷新'}));
  await waitFor(()=>expect(screen.queryByRole('button',{name:'确认应用更改'})).not.toBeInTheDocument());
  await screen.findByText('+updated text');
  expect(screen.queryByText('+new text')).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole('button',{name:'同步选中文件'}));
  fireEvent.click(screen.getByRole('button',{name:'确认应用更改'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('workspace_apply',{workspaceId:'w',paths:['a.rs'],reviewed:[{...reviewed,afterHash:'updated',afterText:'updated text'}]}));
 });
});
describe('右侧面板',()=>{
 it('顶栏“查看更改”开关面板：默认勾选非冲突文件，展开懒加载行级差异，Escape 关闭',async()=>{
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name:string,args:any)=>{
   if(name==='workspace_diff')return [{path:'a.rs',kind:'modified',conflict:false,beforeHash:'b',afterHash:'a',sourceHash:null,beforeText:'old',afterText:'new'},{path:'conf.rs',kind:'modified',conflict:true,beforeHash:null,afterHash:null,sourceHash:null,beforeText:null,afterText:null}];
   if(name==='workspace_file_diff')return {path:'a.rs',unified:'--- a/a.rs\n+++ b/a.rs\n@@ -1,1 +1,1 @@\n-new\n+new text\n'};
   return original(name,args);
  });
  render(<App/>);
  await screen.findByText('内容 a');
  expect(screen.queryByRole('complementary',{name:'文件面板'})).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole('button',{name:'查看更改'}));
  const panel=screen.getByRole('complementary',{name:'文件面板'});
  await within(panel).findByText('a.rs');
  expect(within(panel).getByLabelText('选择 conf.rs')).toBeDisabled();
  fireEvent.click(within(panel).getByRole('button',{name:/a\.rs$/}));
  await within(panel).findByText('+new text');
  fireEvent.keyDown(document,{key:'Escape'});
  expect(screen.queryByRole('complementary',{name:'文件面板'})).not.toBeInTheDocument();
  expect(screen.getByRole('button',{name:'查看更改'})).toHaveAttribute('aria-expanded','false');
 });
 it('write 工具卡的文件按钮切到预览 tab 显示内容',async()=>{
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name:string,args:any)=>{
   if(name==='session_events')return [{id:'w1',sessionId:'a',seq:'1',kind:'tool.result',data:{name:'write',arguments:{path:'src/main.rs'},content:'已写入副本 src/main.rs'},createdAt:'2026-11-07T00:00:00Z'}];
   if(name==='workspace_file_preview')return {path:'src/main.rs',content:'fn main(){}',truncated:false,binary:false,sizeBytes:11};
   return original(name,args);
  });
  render(<App/>);
  await screen.findByRole('button',{name:'main.rs'});
  fireEvent.click(screen.getByRole('button',{name:'main.rs'}));
  await screen.findByText('fn main(){}');
  expect(screen.getByText('src/main.rs')).toBeInTheDocument();
  expect(screen.getByRole('tab',{name:'预览'})).toHaveAttribute('aria-selected','true');
 });
 it('/diff 斜杠命令打开更改视图',async()=>{
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name:string,args:any)=>name==='workspace_diff'?[]:original(name,args));
  render(<App/>);
  await screen.findByText('内容 a');
  const input=screen.getByLabelText('消息');
  fireEvent.change(input,{target:{value:'/diff'}});
  fireEvent.keyDown(input,{key:'Enter'});
  await screen.findByText('工作副本没有待同步更改');
  expect(screen.getByRole('complementary',{name:'文件面板'})).toBeInTheDocument();
 });
});
describe('事件状态',()=>{
 it('重复增量去重，会话隔离，持久化seq按大整数排序',()=>{const state=useChat.getState();const delta:LiveEvent={type:'delta',sessionId:'a',requestId:'r',index:0,text:'A',reasoning:false};state.receive(delta);state.receive(delta);state.receive({...delta,index:1,text:'B'});state.receive({...delta,sessionId:'b',text:'C'});expect(useChat.getState().streams.a.text).toBe('AB');expect(useChat.getState().streams.b.text).toBe('C');state.replace('a',[event('later','a','9007199254740993','later'),event('first','a','9007199254740992','first')]);expect(useChat.getState().events.a.map(e=>e.id)).toEqual(['first','later']);state.receive({type:'durable',event:event('final','a','9007199254740994','AB')});expect(useChat.getState().streams.a).toBeUndefined()});
});

const jobEvents:SessionEvent[]=[
 {id:'s1',sessionId:'a',seq:'1',kind:'job.started',data:{id:'job-1',command:'cargo test --all-features',status:'running'},createdAt:'2026-11-06T01:00:00Z'},
 {id:'c1',sessionId:'a',seq:'2',kind:'job.cancel_requested',data:{id:'job-1'},createdAt:'2026-11-06T01:01:00Z'},
 {id:'f1',sessionId:'a',seq:'3',kind:'job.finished',data:{id:'job-1',status:'cancelled',error:'已取消'},createdAt:'2026-11-06T01:01:05Z'},
 {id:'s2',sessionId:'a',seq:'4',kind:'job.started',data:{id:'job-2',command:'npm run build',status:'running'},createdAt:'2026-11-06T01:02:00Z'},
 {id:'f2',sessionId:'a',seq:'5',kind:'job.finished',data:{id:'job-2',status:'completed',output:{stdout:'built ok',stderr:'',exit_code:0}},createdAt:'2026-11-06T01:03:00Z'},
 {id:'sc1',sessionId:'a',seq:'6',kind:'schedule.created',data:{id:'sch-1',at:'2099-01-01T00:00:00Z',task:'以后执行',modelId:'missing',status:'pending'},createdAt:'2026-11-06T01:04:00Z'},
 {id:'sf1',sessionId:'a',seq:'7',kind:'schedule.finished',data:{id:'sch-1',status:'completed'},createdAt:'2026-11-06T01:05:00Z'},
 {id:'sa1',sessionId:'a',seq:'8',kind:'subagent.created',data:{sessionId:'child-1',source:'a'},createdAt:'2026-11-06T01:06:00Z'},
 {id:'sm1',sessionId:'a',seq:'9',kind:'subagent.message_sent',data:{sessionId:'child-1',content:'真实子任务'},createdAt:'2026-11-06T01:07:00Z'},
];
describe('后台任务可视化',()=>{
 it('消息流渲染后台任务、定时任务与子代理事件卡片',async()=>{
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>name==='session_events'?jobEvents:original(name,args));
  render(<App/>);
  await screen.findByText('npm run build');
  expect(screen.getAllByText('cargo test --all-features').length).toBeGreaterThan(0);
  expect(screen.getByText('已取消')).toBeInTheDocument();
  expect(screen.getByText(/已请求取消后台任务/)).toBeInTheDocument();
  expect(screen.getByText(/built ok/)).toBeInTheDocument();
  expect(screen.getByText('已创建子代理')).toBeInTheDocument();
  expect(screen.getByText('真实子任务')).toBeInTheDocument();
 });
 it('渲染定时任务创建卡片与执行状态行，historyOnly 子代理不渲染',async()=>{
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>name==='session_events'?[...jobEvents,{id:'sa2',sessionId:'a',seq:'10',kind:'subagent.created',data:{sessionId:'ghost',source:'a',historyOnly:true},createdAt:'2026-11-06T01:08:00Z'}]:original(name,args));
  render(<App/>);
  await screen.findByText('以后执行');
  expect(screen.getByText('2099-01-01T00:00:00Z')).toBeInTheDocument();
  expect(screen.getByText(/定时任务 sch-1：已完成/)).toBeInTheDocument();
  expect(screen.queryByText('ghost')).not.toBeInTheDocument();
 });
 it('顶栏按钮统计未终结的后台任务并以浮层开关面板',async()=>{
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>{
   if(name==='session_events')return jobEvents.filter(e=>['s1','f1','s2'].includes(e.id));
   if(name==='job_list')return [];
   if(name==='schedule_list')return [];
   return original(name,args);
  });
  render(<App/>);
  await screen.findByText('npm run build');
  const button=screen.getByRole('button',{name:/后台任务/});
  expect(button).toHaveTextContent('1');
  fireEvent.click(button);
  const panel=await screen.findByRole('region',{name:'后台任务'});
  expect(button).toHaveAttribute('aria-expanded','true');
  expect(mocks.command).toHaveBeenCalledWith('job_list',{sessionId:'a'});
  fireEvent.click(button);
  expect(panel).not.toBeInTheDocument();
  expect(button).toHaveAttribute('aria-expanded','false');
 });
});

const terminalEvents:SessionEvent[]=[
 {id:'t1',sessionId:'a',seq:'1',kind:'terminal.opened',data:{id:'term-1',shell:'pwsh',status:'running'},createdAt:'2026-11-06T02:00:00Z'},
 {id:'t2',sessionId:'a',seq:'2',kind:'terminal.closed',data:{id:'term-1',status:'exited',exitCode:0,outputBlobHash:'blob-hash-1'},createdAt:'2026-11-06T02:01:00Z'},
 {id:'t3',sessionId:'a',seq:'3',kind:'terminal.closed',data:{id:'term-2',status:'interrupted',error:'应用中断，未重放终端副作用'},createdAt:'2026-11-06T02:02:00Z'},
];
describe('终端事件可视化',()=>{
 it('消息流渲染终端打开与关闭卡片',async()=>{
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>name==='session_events'?terminalEvents:original(name,args));
  render(<App/>);
  await screen.findByText(/已打开交互式终端 term-1/);
  expect(screen.getByText(/交互式终端已结束 term-1/)).toBeInTheDocument();
  expect(screen.getByText('已退出')).toBeInTheDocument();
  expect(screen.getByText('已中断')).toBeInTheDocument();
  expect(screen.getByText(/blob-hash-1/)).toBeInTheDocument();
 });
});


describe('工作区的首次会话',()=>{
 it('选择没有会话的新工作区后可直接点击输入并发送',async()=>{
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>{
   if(name==='workspace_list')return [];
   if(name==='workspace_open')return {id:'fresh',sourcePath:'D:/fresh',replicaPath:'D:/copy-fresh'};
   if(name==='session_list')return [];
   if(name==='session_create')return {id:'first',workspaceId:'fresh',title:'新会话'};
   if(name==='session_events')return [];
   return original(name,args);
  });
  vi.mocked(open).mockResolvedValueOnce('D:/fresh');
  render(<App/>);await screen.findByLabelText('当前模型');
  expect(screen.getByLabelText('消息')).toBeDisabled();
  fireEvent.click(screen.getByRole('button',{name:/打开工作区/}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('session_list',{workspaceId:'fresh',allWorkspaces:true}));
  await waitFor(()=>expect(screen.getByLabelText('消息')).toBeEnabled());
  const input=screen.getByLabelText('消息');input.focus();expect(input).toHaveFocus();
  fireEvent.change(input,{target:{value:'梳理项目结构'}});
  expect(input).toHaveValue('梳理项目结构');
  fireEvent.click(screen.getByRole('button',{name:'发送消息'}));
  await waitFor(()=>expect(mocks.send).toHaveBeenCalledWith('first','梳理项目结构','m',expect.any(Function),undefined,'high'));
  expect(mocks.command.mock.calls.filter(([name])=>name==='session_create')).toHaveLength(1);
 });
 it('启动时恢复的工作区没有会话也会自动准备首个会话',async()=>{
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>{
   if(name==='session_list')return [];
   if(name==='session_create')return {id:'restored',workspaceId:'w',title:'新会话'};
   if(name==='session_events')return [];
   return original(name,args);
  });
  render(<App/>);
  await waitFor(()=>expect(screen.getByLabelText('消息')).toBeEnabled());
  expect(mocks.command).toHaveBeenCalledWith('session_create',{workspaceId:'w',title:'新会话'});
 });
 it('自动创建失败会显示错误，并允许点击新建会话重试',async()=>{
  let fail=true;
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>{
   if(name==='session_list')return [];
   if(name==='session_create'){if(fail)throw new Error('创建会话失败');return {id:'retry',workspaceId:'w',title:'新会话'};}
   if(name==='session_events')return [];
   return original(name,args);
  });
  render(<App/>);await screen.findByText('创建会话失败');
  expect(screen.getByLabelText('消息')).toBeDisabled();
  expect(screen.getByRole('button',{name:'新建会话'})).toBeEnabled();
  fail=false;fireEvent.click(screen.getByRole('button',{name:'新建会话'}));
  await waitFor(()=>expect(screen.getByLabelText('消息')).toBeEnabled());
  expect(screen.queryByText('创建会话失败')).not.toBeInTheDocument();
 });
 it('首次会话的迟到响应不能覆盖已切换到的工作区',async()=>{
  let resolveCreation!:(value:unknown)=>void;
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>{
   if(name==='workspace_list')return [{id:'w',sourcePath:'D:/A'},{id:'other',sourcePath:'D:/B'}];
   if(name==='session_list')return args.workspaceId==='w'?[]:[{id:'other-session',workspaceId:'other',title:'B 的会话'}];
   if(name==='session_create')return new Promise(resolve=>{resolveCreation=resolve});
   return original(name,args);
  });
  render(<App/>);
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('session_create',{workspaceId:'w',title:'新会话'}));
  expect(screen.getByRole('button',{name:'新建会话'})).toBeDisabled();
  fireEvent.change(screen.getByLabelText('工作区'),{target:{value:'other'}});
  await screen.findByText('内容 other-session');
  await act(async()=>resolveCreation({id:'late',workspaceId:'w',title:'A 迟到的会话'}));
  expect(screen.queryByRole('button',{name:'A 迟到的会话'})).not.toBeInTheDocument();
  expect(screen.getByLabelText('消息')).toBeEnabled();
  fireEvent.change(screen.getByLabelText('消息'),{target:{value:'给 B'}});
  fireEvent.click(screen.getByRole('button',{name:'发送消息'}));
  await waitFor(()=>expect(mocks.send).toHaveBeenCalledWith('other-session','给 B','m',expect.any(Function),undefined,'high'));
 });
 it('快速切走再返回时复用正在创建的首个会话',async()=>{
  let resolveCreation!:(value:unknown)=>void;
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>{
   if(name==='workspace_list')return [{id:'w',sourcePath:'D:/A'},{id:'other',sourcePath:'D:/B'}];
   if(name==='session_list')return args.workspaceId==='w'?[]:[{id:'other-session',workspaceId:'other',title:'B 的会话'}];
   if(name==='session_create')return new Promise(resolve=>{resolveCreation=resolve});
   return original(name,args);
  });
  render(<App/>);
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('session_create',{workspaceId:'w',title:'新会话'}));
  fireEvent.change(screen.getByLabelText('工作区'),{target:{value:'other'}});
  await screen.findByText('内容 other-session');
  fireEvent.change(screen.getByLabelText('工作区'),{target:{value:'w'}});
  await waitFor(()=>expect(mocks.command.mock.calls.filter(([name,args])=>name==='session_list'&&args.workspaceId==='w')).toHaveLength(2));
  await act(async()=>resolveCreation({id:'first',workspaceId:'w',title:'A 的首个会话'}));
  await screen.findByText('内容 first');
  expect(screen.getByLabelText('消息')).toBeEnabled();
  expect(mocks.command.mock.calls.filter(([name])=>name==='session_create')).toHaveLength(1);
 });
});


describe('附件与完整输出',()=>{
 const attachment={hash:'a'.repeat(64),name:'说明.txt',mediaType:'text/plain',size:12};
 it('通过加号菜单选择附件后只将宿主返回的hash随消息发送',async()=>{
  const original=mocks.command.getMockImplementation()!;mocks.command.mockImplementation(async(name,args)=>name==='artifact_import'?attachment:original(name,args));vi.mocked(open).mockResolvedValueOnce('C:/private/说明.txt');
  render(<App/>);await screen.findByText('内容 a');fireEvent.click(screen.getByRole('button',{name:'添加内容'}));fireEvent.click(screen.getByRole('menuitem',{name:/^附件/}));await screen.findByText('说明.txt');fireEvent.change(screen.getByLabelText('消息'),{target:{value:'读取附件'}});fireEvent.click(screen.getByRole('button',{name:'发送消息'}));
  await waitFor(()=>expect(mocks.send).toHaveBeenCalledWith('a','读取附件','m',expect.any(Function),[attachment.hash],'high'));expect(mocks.command).toHaveBeenCalledWith('artifact_import',{sessionId:'a',path:'C:/private/说明.txt'});
 });
 it('导入期间禁止发送，切换会话后丢弃迟到的附件',async()=>{
  let finish!:(value:unknown)=>void;const original=mocks.command.getMockImplementation()!;mocks.command.mockImplementation(async(name,args)=>name==='artifact_import'?new Promise(resolve=>finish=resolve):original(name,args));vi.mocked(open).mockResolvedValueOnce('C:/private/说明.txt');
  render(<App/>);await screen.findByText('内容 a');fireEvent.change(screen.getByLabelText('消息'),{target:{value:'任务'}});fireEvent.click(screen.getByRole('button',{name:'添加内容'}));fireEvent.click(screen.getByRole('menuitem',{name:/^附件/}));await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('artifact_import',expect.anything()));expect(screen.getByRole('button',{name:'发送消息'})).toBeDisabled();fireEvent.click(screen.getByRole('button',{name:/会话 B/}));await screen.findByText('内容 b');await act(async()=>finish(attachment));expect(screen.queryByText('说明.txt')).not.toBeInTheDocument();
  fireEvent.change(screen.getByLabelText('消息'),{target:{value:'无附件'}});fireEvent.click(screen.getByRole('button',{name:'发送消息'}));await waitFor(()=>expect(mocks.send).toHaveBeenCalledWith('b','无附件','m',expect.any(Function),undefined,'high'));
 });
 it('附件和大输出卡片通过保存对话框导出真实完整内容',async()=>{
  const original=mocks.command.getMockImplementation()!;mocks.command.mockImplementation(async(name,args)=>name==='session_events'?[{id:'attached',sessionId:'a',seq:'1',kind:'user.message',data:{content:'查看',attachments:[attachment]},createdAt:''},{id:'output',sessionId:'a',seq:'2',kind:'tool.result',data:{name:'read',content:'预览',artifact:{...attachment,name:'完整输出.txt'}},createdAt:''}]:original(name,args));vi.mocked(save).mockResolvedValueOnce('C:/saved/output.txt');
  render(<App/>);await screen.findByRole('button',{name:'下载 完整输出.txt'});fireEvent.click(screen.getByRole('button',{name:'下载 完整输出.txt'}));await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('artifact_save',{sessionId:'a',hash:attachment.hash,path:'C:/saved/output.txt'}));expect(screen.getByRole('button',{name:'下载 说明.txt'})).toBeInTheDocument();
 });
});

describe('子代理与任务状态可视化',()=>{
 it('子代理事件渲染卡片，跳转子会话并显示运行状态与子任务徽标',async()=>{
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>{
   if(name==='session_list')return [{id:'a',workspaceId:'w',title:'父会话'},{id:'child',workspaceId:'w',title:'依赖梳理子任务',parentId:'a'}];
   if(name==='session_active_list')return ['child'];
   if(name==='session_events')return args.sessionId==='a'?[
    {id:'s1',sessionId:'a',seq:'1',kind:'subagent.created',data:{sessionId:'child'},createdAt:''},
    {id:'s2',sessionId:'a',seq:'2',kind:'subagent.message_sent',data:{sessionId:'child',content:'梳理依赖'},createdAt:''}
   ]:[event(`e-${args.sessionId}`,args.sessionId,'1',`内容 ${args.sessionId}`)];
   return original(name,args);
  });
  render(<App/>);
  await screen.findByText('已创建子代理');
  expect(screen.getByText('已向子代理投递任务')).toBeInTheDocument();
  expect(screen.getByText('梳理依赖')).toBeInTheDocument();
  expect(screen.getAllByLabelText('子代理运行中').length).toBeGreaterThan(0);
  expect(screen.getByText('子任务')).toBeInTheDocument();
  fireEvent.click(screen.getAllByRole('button',{name:'查看会话'})[0]);
  await screen.findByText('内容 child');
 });
 it('fork 副本中的历史派生记录不提供跳转',async()=>{
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>name==='session_events'?[
   {id:'s1',sessionId:'a',seq:'1',kind:'subagent.created',data:{sessionId:'child',historyOnly:true},createdAt:''}
  ]:original(name,args));
  render(<App/>);
  await screen.findByText('历史派生记录，原父子关系属于源会话。');
  expect(screen.queryByRole('button',{name:'查看会话'})).not.toBeInTheDocument();
 });
 it('todo与plan状态事件渲染为清单与文本卡片',async()=>{
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>name==='session_events'?[
   {id:'t1',sessionId:'a',seq:'1',kind:'todo.state',data:{items:[{text:'已完成项',done:true},{text:'待办项'}]},createdAt:''},
   {id:'p1',sessionId:'a',seq:'2',kind:'plan.state',data:{content:'## 分三步走'},createdAt:''}
  ]:original(name,args));
  const {container}=render(<App/>);
  await screen.findByText('待办清单');
  expect(container.querySelector('.todo-list li.done')?.textContent).toContain('已完成项');
  expect(screen.getByText(/待办项/)).toBeInTheDocument();
  expect(screen.getByRole('heading',{name:'分三步走',level:2})).toBeInTheDocument();
 });
});
describe('计划与目标面板',()=>{
 it('侧边栏入口打开面板并跨会话汇总，可跳转同工作区会话',async()=>{
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>name==='plan_overview'?[
   {sessionId:'a',sessionTitle:'会话 A',workspaceId:'w',kind:'todo.state',data:{items:[{text:'修复登录',done:true},{text:'补充测试'}]},updatedAt:'2026-11-06T10:00:00Z'},
   {sessionId:'a',sessionTitle:'会话 A',workspaceId:'w',kind:'plan.state',data:{content:'## 先写测试'},updatedAt:'2026-11-06T09:00:00Z'}
  ]:original(name,args));
  const {container}=render(<App/>);
  await screen.findByText('内容 a');
  fireEvent.click(screen.getByRole('button',{name:'计划与目标'}));
  const dialog=await screen.findByRole('dialog',{name:'计划与目标'});
  expect(within(dialog).getByText('会话 A')).toBeInTheDocument();
  expect(container.querySelector('.todo-list li.done')?.textContent).toContain('修复登录');
  expect(container.querySelector('.todo-list li:not(.done)')?.textContent).toContain('补充测试');
  expect(within(dialog).getByRole('heading',{name:'先写测试',level:2})).toBeInTheDocument();
  fireEvent.click(within(dialog).getByRole('button',{name:'当前会话'}));
  await waitFor(()=>expect(screen.queryByRole('dialog',{name:'计划与目标'})).not.toBeInTheDocument());
 });
});
describe('模型选择',()=>{
 const builtinCatalog=[
  {id:'deepseek',name:'DeepSeek',builtin:true,compatibility:'deepSeek',endpoints:[{apiMode:'anthropic',baseUrl:'https://api.deepseek.com/anthropic'}],defaultModel:'deepseek-flash',credentialRef:'',proxy:null,enabled:true,builtinModels:[{id:'deepseek-flash',name:'DeepSeek Flash',efforts:['disabled','enabled']}],disabledModels:[]},
  {id:'bigmodel',name:'智谱 Coding Plan',builtin:true,compatibility:'bigModel',endpoints:[{apiMode:'anthropic',baseUrl:'https://open.bigmodel.cn/api/anthropic'}],defaultModel:'glm-5.3',credentialRef:'',proxy:null,enabled:true,builtinModels:[{id:'glm-5.3',name:'GLM-5.3',efforts:['low','high','max']}],disabledModels:['glm-5.3']},
 ];
 const models=[
  {id:'builtin:deepseek:deepseek-flash',name:'DeepSeek Flash',compatibility:'deepSeek',providerId:'deepseek',apiMode:'anthropic',maxOutputTokens:null,baseUrl:'https://api.deepseek.com/anthropic',model:'deepseek-flash',credentialRef:'',proxy:null,efforts:['disabled','enabled'],effort:null,enabled:true,builtin:true},
  {id:'custom',name:'自定义 GLM',compatibility:'bigModel',providerId:'bigmodel',apiMode:'anthropic',maxOutputTokens:null,baseUrl:'https://open.bigmodel.cn/api/anthropic',model:'glm-5.3',credentialRef:'',proxy:null,efforts:['low','high','max'],effort:'high',enabled:true,builtin:false},
  {id:'builtin:bigmodel:glm-5.3',name:'GLM-5.3',compatibility:'bigModel',providerId:'bigmodel',apiMode:'anthropic',maxOutputTokens:null,baseUrl:'https://open.bigmodel.cn/api/anthropic',model:'glm-5.3',credentialRef:'',proxy:null,efforts:['low','high','max'],effort:null,enabled:false,builtin:true},
 ];
 function modelApp(){const original=mocks.command.getMockImplementation()!;mocks.command.mockImplementation(async(name:string,args:any)=>{
  if(name==='provider_list')return structuredClone(builtinCatalog);
  if(name==='model_list')return structuredClone(models);
  if(name==='default_selection_get')return null;
  if(name==='default_selection_save')return null;
  return original(name,args);
 })}
 it('触发按钮显示当前模型与档位徽标，点击展开浮层卡片',async()=>{
  modelApp();render(<App/>);
  const trigger=await screen.findByLabelText('当前模型');
  expect(trigger).toHaveTextContent('DeepSeek Flash');
  // 默认档位即模型最高档
  expect(within(trigger).getByText('enabled')).toBeInTheDocument();
  fireEvent.click(trigger);
  const menu=screen.getByRole('listbox',{name:'选择模型'});
  expect(within(menu).getByText('DeepSeek')).toBeInTheDocument();
  expect(within(menu).getByText('智谱 Coding Plan')).toBeInTheDocument();
  // 停用模型灰显展示并标注原因，点击不产生任何选择
  const disabledRow=within(menu).getByText('GLM-5.3').closest('button')!;
  expect(disabledRow.className).toContain('disabled');
  expect(within(disabledRow).getByText('已停用')).toBeInTheDocument();
  fireEvent.click(disabledRow);
  expect(mocks.command).not.toHaveBeenCalledWith('default_selection_save',expect.anything());
  // 档位滑块存在，当前档位为最高档
  expect(screen.getByLabelText('推理档位')).toHaveAttribute('aria-valuetext','enabled');
 });
 it('搜索过滤模型；选模型保持卡片展开并持久化，档位回落最高档；会话流出现临时切换标记',async()=>{
  mocks.send.mockResolvedValue(undefined);
  modelApp();render(<App/>);
  await screen.findByText('内容 a');
  fireEvent.click(screen.getByLabelText('当前模型'));
  fireEvent.change(screen.getByLabelText('搜索模型'),{target:{value:'自定义'}});
  const menu=screen.getByRole('listbox',{name:'选择模型'});
  expect(within(menu).queryByText('DeepSeek Flash')).not.toBeInTheDocument();
  fireEvent.click(within(menu).getByText('自定义 GLM'));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('default_selection_save',{selection:{modelId:'custom',effort:'max'}}));
  // 卡片保持展开，滑块区已切换为新模型档位，可直接继续调档
  expect(screen.getByRole('listbox',{name:'选择模型'})).toBeInTheDocument();
  expect(screen.getByLabelText('推理档位')).toHaveAttribute('aria-valuetext','max');
  fireEvent.change(screen.getByLabelText('推理档位'),{target:{value:'1'}});
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('default_selection_save',{selection:{modelId:'custom',effort:'high'}}));
  // 会话流插入临时切换标记（不落库）
  expect(screen.getByText(/模型已切换：DeepSeek Flash → 自定义 GLM · 档位 max/)).toBeInTheDocument();
  fireEvent.change(screen.getByLabelText('消息'),{target:{value:'你好'}});
  fireEvent.click(screen.getByRole('button',{name:'发送消息'}));
  await waitFor(()=>expect(mocks.send).toHaveBeenCalledWith('a','你好','custom',expect.any(Function),undefined,'high'));
 });
 it('点击当前模型行关闭卡片，Esc 亦可关闭',async()=>{
  modelApp();render(<App/>);
  fireEvent.click(await screen.findByLabelText('当前模型'));
  fireEvent.click(within(screen.getByRole('listbox',{name:'选择模型'})).getByText('DeepSeek Flash'));
  expect(screen.queryByRole('listbox',{name:'选择模型'})).not.toBeInTheDocument();
  fireEvent.click(screen.getByLabelText('当前模型'));
  fireEvent.keyDown(screen.getByLabelText('搜索模型'),{key:'Escape'});
  expect(screen.queryByRole('listbox',{name:'选择模型'})).not.toBeInTheDocument();
 });
 it('启动恢复上次选择；失效选择回退首个可用模型并将档位归到最高档',async()=>{
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name:string,args:any)=>{
   if(name==='model_list')return structuredClone(models);
   if(name==='provider_list')return structuredClone(builtinCatalog);
   if(name==='default_selection_get')return {modelId:'missing-model',effort:'max'};
   return original(name,args);
  });
  render(<App/>);
  const trigger=await screen.findByLabelText('当前模型');
  expect(trigger).toHaveTextContent('DeepSeek Flash');
  expect(within(trigger).getByText('enabled')).toBeInTheDocument();
 });
 it('无档位模型隐藏档位徽标与滑块区',async()=>{
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name:string,args:any)=>{
   if(name==='model_list')return [{...structuredClone(models[0]),efforts:[],effort:null}];
   if(name==='provider_list')return structuredClone(builtinCatalog);
   if(name==='default_selection_get')return null;
   return original(name,args);
  });
  render(<App/>);
  const trigger=await screen.findByLabelText('当前模型');
  expect(trigger.querySelector('.model-trigger-effort')).toBeNull();
  fireEvent.click(trigger);
  expect(screen.queryByLabelText('推理档位')).not.toBeInTheDocument();
 });
 it('/model 斜杠命令打开浮层卡片',async()=>{
  modelApp();render(<App/>);await screen.findByText('内容 a');
  const input=screen.getByLabelText('消息');
  fireEvent.change(input,{target:{value:'/model'}});
  fireEvent.keyDown(input,{key:'Enter'});
  expect(screen.getByRole('listbox',{name:'选择模型'})).toBeInTheDocument();
 });
});
describe('斜杠命令补全',()=>{
 it('输入 / 显示候选菜单，过滤收窄，Esc 关闭后 token 变化重开',async()=>{
  render(<App/>);await screen.findByText('内容 a');
  const input=screen.getByLabelText('消息');
  fireEvent.change(input,{target:{value:'/'}});
  const menu=screen.getByRole('listbox',{name:'斜杠命令补全'});
  expect(within(menu).getByText('/clear')).toBeInTheDocument();
  expect(within(menu).getByText('/help')).toBeInTheDocument();
  fireEvent.change(input,{target:{value:'/he'}});
  expect(within(screen.getByRole('listbox',{name:'斜杠命令补全'})).getByText('/help')).toBeInTheDocument();
  expect(screen.queryByText('/clear')).not.toBeInTheDocument();
  fireEvent.keyDown(input,{key:'Escape'});
  expect(screen.queryByRole('listbox',{name:'斜杠命令补全'})).not.toBeInTheDocument();
  fireEvent.change(input,{target:{value:'/hel'}});
  expect(screen.getByRole('listbox',{name:'斜杠命令补全'})).toBeInTheDocument();
 });
 it('ArrowDown 循环选择，Tab 补全为规范名',async()=>{
  render(<App/>);await screen.findByText('内容 a');
  const input=screen.getByLabelText('消息');
  fireEvent.change(input,{target:{value:'/c'}});
  const options=within(screen.getByRole('listbox',{name:'斜杠命令补全'})).getAllByRole('option');
  expect(options.map(option=>option.textContent)).toEqual(['/clear重开新会话','/compact压缩历史为摘要']);
  expect(options[0]).toHaveAttribute('aria-selected','true');
  fireEvent.keyDown(input,{key:'ArrowDown'});
  expect(options[1]).toHaveAttribute('aria-selected','true');
  fireEvent.keyDown(input,{key:'Tab'});
  expect(input).toHaveValue('/compact ');
 });
 it('Enter 执行选中命令：/clear 重开会话',async()=>{
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name:string,args:any)=>name==='session_create'?{id:'fresh',workspaceId:'w',title:'新会话'}:original(name,args));
  render(<App/>);await screen.findByText('内容 a');
  const input=screen.getByLabelText('消息');
  fireEvent.change(input,{target:{value:'/cl'}});
  fireEvent.keyDown(input,{key:'Enter'});
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('session_create',{workspaceId:'w',title:'新会话'}));
  await screen.findByText('内容 fresh');
 });
 it('光标移出首 token 后菜单关闭，Enter 走普通发送',async()=>{
  mocks.send.mockResolvedValue(undefined);
  render(<App/>);await screen.findByText('内容 a');
  const input=screen.getByLabelText('消息');
  fireEvent.change(input,{target:{value:'/clear 帮我重开'}});
  expect(screen.queryByRole('listbox',{name:'斜杠命令补全'})).not.toBeInTheDocument();
  fireEvent.keyDown(input,{key:'Enter'});
  await waitFor(()=>expect(mocks.send).toHaveBeenCalledWith('a','/clear 帮我重开','m',expect.any(Function),undefined,'high'));
 });
});

describe('加号添加菜单',()=>{
 it('点 + 弹出菜单，计划/目标插入斜杠命令并关闭菜单',async()=>{
  render(<App/>);await screen.findByText('内容 a');
  const plus=screen.getByRole('button',{name:'添加内容'});
  expect(screen.queryByRole('menu',{name:'添加内容'})).not.toBeInTheDocument();
  fireEvent.click(plus);
  const menu=screen.getByRole('menu',{name:'添加内容'});
  expect(plus).toHaveAttribute('aria-expanded','true');
  expect(within(menu).getByRole('menuitem',{name:/^附件/})).toBeInTheDocument();
  fireEvent.click(within(menu).getByRole('menuitem',{name:/^计划/}));
  expect(screen.getByLabelText('消息')).toHaveValue('/plan ');
  expect(screen.queryByRole('menu',{name:'添加内容'})).not.toBeInTheDocument();
  expect(screen.getByLabelText('消息')).toHaveFocus();
  fireEvent.click(plus);
  fireEvent.click(screen.getByRole('menuitem',{name:/^目标/}));
  expect(screen.getByLabelText('消息')).toHaveValue('/goal ');
 });
 it('Escape 与点击外部关闭菜单',async()=>{
  render(<App/>);await screen.findByText('内容 a');
  fireEvent.click(screen.getByRole('button',{name:'添加内容'}));
  expect(screen.getByRole('menu',{name:'添加内容'})).toBeInTheDocument();
  fireEvent.keyDown(screen.getByLabelText('消息'),{key:'Escape'});
  expect(screen.queryByRole('menu',{name:'添加内容'})).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole('button',{name:'添加内容'}));
  fireEvent.mouseDown(screen.getByRole('button',{name:'新建会话'}));
  expect(screen.queryByRole('menu',{name:'添加内容'})).not.toBeInTheDocument();
 });
 it('/plan 带参数发送展开为预置提示词，无参数提示用法且不发送',async()=>{
  mocks.send.mockResolvedValue(undefined);
  render(<App/>);await screen.findByText('内容 a');
  const input=screen.getByLabelText('消息');
  fireEvent.change(input,{target:{value:'/plan 为导出制定计划'}});
  fireEvent.keyDown(input,{key:'Enter'});
  await waitFor(()=>expect(mocks.send).toHaveBeenCalledWith('a',`${PLAN_PROMPT}\n\n任务：为导出制定计划`,'m',expect.any(Function),undefined,'high'));
  fireEvent.change(input,{target:{value:'/plan'}});
  fireEvent.keyDown(input,{key:'Enter'});
  await screen.findByText('用法：/plan <任务描述>，发送后模型制定实施计划并经 plan_write 记录。');
  expect(mocks.send).toHaveBeenCalledTimes(1);
 });
 it('/goal 带参数发送展开为预置提示词',async()=>{
  mocks.send.mockResolvedValue(undefined);
  render(<App/>);await screen.findByText('内容 a');
  const input=screen.getByLabelText('消息');
  fireEvent.change(input,{target:{value:'/goal 完成导出功能'}});
  fireEvent.keyDown(input,{key:'Enter'});
  await waitFor(()=>expect(mocks.send).toHaveBeenCalledWith('a',`${GOAL_PROMPT}\n\n目标：完成导出功能`,'m',expect.any(Function),undefined,'high'));
 });
});
describe('能力预设',()=>{
 it('发送框选择预设填充级联，手改级联视为放弃预设',async()=>{
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name:string,args:any)=>{
   if(name==='preset_list')return [{id:'p1',name:'快速响应',description:null,modelId:'m',effort:'high',approval:'auto',broken:null},{id:'stale',name:'失效预设',description:null,modelId:'ghost',effort:null,approval:null,broken:'模型 ghost 不存在'}];
   return original(name,args);
  });
  render(<App/>);await screen.findByText('内容 a');
  const presetSelect=screen.getByLabelText('能力预设') as HTMLSelectElement;
  expect(presetSelect.value).toBe('');
  const staleOption=screen.getByRole('option',{name:/失效预设/}) as HTMLOptionElement;
  expect(staleOption.disabled).toBe(true);
  fireEvent.change(presetSelect,{target:{value:'p1'}});
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('default_selection_save',{selection:{modelId:'m',effort:'high'}}));
  // 打开浮层卡片查看档位滑块（默认档位即最高档）
  fireEvent.click(screen.getByLabelText('当前模型'));
  expect(screen.getByLabelText('推理档位')).toHaveAttribute('aria-valuetext','high');
  // 手改档位即放弃预设
  fireEvent.change(screen.getByLabelText('推理档位'),{target:{value:'0'}});
  expect((screen.getByLabelText('能力预设') as HTMLSelectElement).value).toBe('');
  expect(mocks.command).toHaveBeenCalledWith('default_selection_save',{selection:{modelId:'m',effort:'low'}});
 });
});

describe('会话列表视图',()=>{
 function installSessionMocks(){
  mocks.command.mockImplementation(async(name:string,args:any)=>{
   if(name==='workspace_list')return [
    {id:'w',sourcePath:'D:/alpha',replicaPath:'D:/copy-a',createdAt:'',pinned:false},
    {id:'other',sourcePath:'D:/beta',replicaPath:'D:/copy-b',createdAt:'',pinned:false}];
   if(name==='session_list')return [
    {id:'a',workspaceId:'w',title:'会话 A',createdAt:'',updatedAt:new Date().toISOString(),parentId:null},
    {id:'b',workspaceId:'other',title:'B 的会话',createdAt:'',updatedAt:new Date().toISOString(),parentId:null}];
   if(name==='workspace_set_pinned')return {id:args.workspaceId,sourcePath:'D:/alpha',replicaPath:'D:/copy-a',createdAt:'',pinned:args.pinned};
   if(name==='session_events')return [event(`e-${args.sessionId}`,args.sessionId,'1',`内容 ${args.sessionId}`)];
   return [];
  });
 }
 it('「最近」视图按时间分组并展示跨工作区会话，switch 切到「项目」按工作区分组',async()=>{
  localStorage.clear();installSessionMocks();
  render(<App/>);await screen.findByText('内容 a');
  // 最近视图：今天分组下同时出现两个项目的会话
  expect(screen.getByText('今天')).toBeInTheDocument();
  expect(screen.getByRole('button',{name:/会话 A/})).toBeInTheDocument();
  expect(screen.getByRole('button',{name:/B 的会话/})).toBeInTheDocument();
  const sw=screen.getByRole('switch',{name:'切换视图：最近 / 项目'});
  expect(sw).toHaveAttribute('aria-checked','false');
  fireEvent.click(sw);
  expect(sw).toHaveAttribute('aria-checked','true');
  // 项目视图：两个工作区成组，组内含各自会话
  expect(screen.getByRole('button',{name:/^alpha \d+$/})).toBeInTheDocument();
  expect(screen.getByRole('button',{name:/^beta \d+$/})).toBeInTheDocument();
  expect(screen.getAllByRole('button',{name:/会话 A/}).length).toBeGreaterThan(0);
  expect(screen.getAllByRole('button',{name:/B 的会话/}).length).toBeGreaterThan(0);
  expect(localStorage.getItem('sessionView')).toBe('project');
 });
 it('项目组可置顶并调用 workspace_set_pinned，搜索按标题过滤',async()=>{
  localStorage.clear();installSessionMocks();
  render(<App/>);await screen.findByText('内容 a');
  fireEvent.click(screen.getByRole('switch',{name:'切换视图：最近 / 项目'}));
  fireEvent.click(screen.getByRole('button',{name:'置顶 alpha'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('workspace_set_pinned',{workspaceId:'w',pinned:true}));
  // 置顶的 alpha 排在 beta 之前
  const names=screen.getAllByRole('button',{name:/^(alpha|beta) \d+$/}).map(b=>b.textContent);
  expect(names[0]).toContain('alpha');
  // 搜索按标题过滤，无匹配时给出空态
  fireEvent.change(screen.getByLabelText('搜索会话'),{target:{value:'不存在'}});
  expect(screen.getByText('无匹配会话。')).toBeInTheDocument();
  fireEvent.change(screen.getByLabelText('搜索会话'),{target:{value:'B'}});
  expect(screen.queryByRole('button',{name:/会话 A/})).not.toBeInTheDocument();
  expect(screen.getAllByRole('button',{name:/B 的会话/}).length).toBeGreaterThan(0);
 });
 it('置顶失败时回滚乐观更新并显示错误',async()=>{
  localStorage.clear();
  mocks.command.mockImplementation(async(name:string,args:any)=>{
   if(name==='workspace_list')return [{id:'w',sourcePath:'D:/alpha',replicaPath:'D:/copy-a',createdAt:'',pinned:false}];
   if(name==='session_list')return [{id:'a',workspaceId:'w',title:'会话 A',createdAt:'',updatedAt:new Date().toISOString(),parentId:null}];
   if(name==='workspace_set_pinned')throw new Error('写入失败');
   if(name==='session_events')return [event(`e-${args.sessionId}`,args.sessionId,'1',`内容 ${args.sessionId}`)];
   return [];
  });
  render(<App/>);await screen.findByText('内容 a');
  fireEvent.click(screen.getByRole('switch',{name:'切换视图：最近 / 项目'}));
  fireEvent.click(screen.getByRole('button',{name:'置顶 alpha'}));
  await screen.findByText('写入失败');
  expect(screen.getByRole('button',{name:'置顶 alpha'})).toBeInTheDocument();
 });
});
