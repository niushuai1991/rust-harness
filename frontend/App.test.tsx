import {afterEach,beforeEach,describe,expect,it,vi} from 'vitest';
import {act,cleanup,fireEvent,render,screen,waitFor} from '@testing-library/react';
import App,{DiffPanel} from './App';
import {useChat} from './store';
import {open,save} from '@tauri-apps/plugin-dialog';
import type {LiveEvent,SessionEvent} from './types';
const mocks=vi.hoisted(()=>({command:vi.fn(),send:vi.fn()}));
vi.mock('./api',()=>({desktop:()=>true,command:mocks.command,send:mocks.send,errorText:(e:unknown)=>e instanceof Error?e.message:String(e)}));
vi.mock('@tauri-apps/plugin-dialog',()=>({open:vi.fn(),save:vi.fn()}));
const event=(id:string,sessionId:string,seq:string,content:string):SessionEvent=>({id,sessionId,seq,kind:'assistant.message',data:{content},createdAt:''});
beforeEach(()=>{useChat.setState({events:{},streams:{},statuses:{}});mocks.command.mockReset();mocks.send.mockReset();mocks.command.mockImplementation(async(name:string,args:any)=>{
 if(name==='workspace_list')return [{id:'w',sourcePath:'D:/project',replicaPath:'D:/copy',createdAt:''}];
 if(name==='model_list')return [{id:'m',name:'测试模型',compatibility:'standard',providerId:'custom',apiMode:'chatCompletions',maxOutputTokens:null,baseUrl:'https://example.test',model:'m',credentialRef:'',proxy:null}];
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
  await waitFor(()=>expect(mocks.send).toHaveBeenCalledWith('other-session','仅发送给 B','m',expect.any(Function)));
 });
 it('切换会话只显示该会话的消息',async()=>{render(<App/>);await screen.findByText('内容 a');fireEvent.click(screen.getByRole('button',{name:'会话 B'}));await screen.findByText('内容 b');expect(screen.queryByText('内容 a')).not.toBeInTheDocument()});
 it('发送期间合并增量，结束后读取持久化事件',async()=>{let release!:()=>void;mocks.send.mockImplementation(async(_id:string,_text:string,_model:string,receive:(e:LiveEvent)=>void)=>{receive({type:'delta',sessionId:'a',requestId:'r',index:0,text:'正在输出',reasoning:false});await new Promise<void>(r=>release=r)});render(<App/>);await screen.findByText('内容 a');fireEvent.change(screen.getByLabelText('消息'),{target:{value:'你好'}});fireEvent.click(screen.getByRole('button',{name:'发送消息'}));await screen.findByText('正在输出');expect(screen.getByRole('button',{name:'取消生成'})).toBeInTheDocument();release();await waitFor(()=>expect(screen.queryByText('正在输出')).not.toBeInTheDocument());expect(mocks.send).toHaveBeenCalledWith('a','你好','m',expect.any(Function))});
 it('发送错误可见且不插入假成功消息',async()=>{mocks.send.mockRejectedValue(new Error('连接失败'));render(<App/>);await screen.findByText('内容 a');fireEvent.change(screen.getByLabelText('消息'),{target:{value:'请求'}});fireEvent.click(screen.getByRole('button',{name:'发送消息'}));await screen.findByText('连接失败');expect(screen.getByRole('button',{name:'发送消息'})).toBeInTheDocument()});
 it('审批通过传递真实请求id',async()=>{const original=mocks.command.getMockImplementation()!;mocks.command.mockImplementation(async(name,args)=>name==='approval_list'?[{id:'approval-1',sessionId:'a',tool:'shell',arguments:{command:'cargo test'},reason:'执行测试'}]:original(name,args));render(<App/>);await screen.findByRole('dialog',{name:'操作需要你的批准'});fireEvent.click(screen.getByRole('button',{name:'批准此次操作'}));await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('approval_resolve',{id:'approval-1',allow:true}))});
});
describe('同步审阅',()=>{
 it('只应用选中文件，传递完整审阅快照且需要二次确认',async()=>{
  const reviewed={path:'a.rs',kind:'modified',conflict:false,beforeHash:'before-a',afterHash:'reviewed-a',sourceHash:'source-a',beforeText:'old',afterText:'new'};
  mocks.command.mockResolvedValue([reviewed,{...reviewed,path:'b.rs'},{...reviewed,path:'conflict.rs',conflict:true}]);
  render(<DiffPanel workspaceId="w" onClose={()=>{}}/>);await screen.findByText('a.rs');fireEvent.click(screen.getByLabelText('b.rs'));expect(screen.getByLabelText('conflict.rs')).toBeDisabled();fireEvent.click(screen.getByRole('button',{name:'同步选中文件'}));expect(mocks.command).not.toHaveBeenCalledWith('workspace_apply',expect.anything());fireEvent.click(screen.getByRole('button',{name:'确认应用更改'}));await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('workspace_apply',{workspaceId:'w',paths:['a.rs'],reviewed:[reviewed]}));
 });
 it('宿主拒绝过期审阅时保留错误且不声称已同步',async()=>{
  const reviewed={path:'a.rs',kind:'modified',conflict:false,beforeHash:'before',afterHash:'reviewed',sourceHash:'before',beforeText:'old',afterText:'reviewed text'};
  mocks.command.mockImplementation(async name=>{if(name==='workspace_apply')throw new Error('审阅后副本已改变，请重新审阅');return [reviewed]});
  render(<DiffPanel workspaceId="w" onClose={()=>{}}/>);
  await screen.findByText('a.rs');fireEvent.click(screen.getByRole('button',{name:'同步选中文件'}));fireEvent.click(screen.getByRole('button',{name:'确认应用更改'}));
  await screen.findByText('审阅后副本已改变，请重新审阅');
  expect(screen.queryByText('已将选中更改同步到原项目。')).not.toBeInTheDocument();
  expect(screen.getByText('reviewed text')).toBeInTheDocument();
 });
 it('刷新差异后需要重新确认，不能沿用旧审阅确认状态',async()=>{
  const reviewed={path:'a.rs',kind:'modified',conflict:false,beforeHash:'before',afterHash:'reviewed',sourceHash:'before',beforeText:'old',afterText:'reviewed text'};
  mocks.command.mockResolvedValue([reviewed]);render(<DiffPanel workspaceId="w" onClose={()=>{}}/>);
  await screen.findByText('a.rs');fireEvent.click(screen.getByRole('button',{name:'同步选中文件'}));
  expect(screen.getByRole('button',{name:'确认应用更改'})).toBeInTheDocument();
  mocks.command.mockResolvedValue([{...reviewed,afterHash:'updated',afterText:'updated text'}]);
  fireEvent.click(screen.getByRole('button',{name:'刷新'}));await screen.findByText('updated text');
  expect(screen.queryByRole('button',{name:'确认应用更改'})).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole('button',{name:'同步选中文件'}));fireEvent.click(screen.getByRole('button',{name:'确认应用更改'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('workspace_apply',{workspaceId:'w',paths:['a.rs'],reviewed:[{...reviewed,afterHash:'updated',afterText:'updated text'}]}));
 });
});
describe('事件状态',()=>{
 it('重复增量去重，会话隔离，持久化seq按大整数排序',()=>{const state=useChat.getState();const delta:LiveEvent={type:'delta',sessionId:'a',requestId:'r',index:0,text:'A',reasoning:false};state.receive(delta);state.receive(delta);state.receive({...delta,index:1,text:'B'});state.receive({...delta,sessionId:'b',text:'C'});expect(useChat.getState().streams.a.text).toBe('AB');expect(useChat.getState().streams.b.text).toBe('C');state.replace('a',[event('later','a','9007199254740993','later'),event('first','a','9007199254740992','first')]);expect(useChat.getState().events.a.map(e=>e.id)).toEqual(['first','later']);state.receive({type:'durable',event:event('final','a','9007199254740994','AB')});expect(useChat.getState().streams.a).toBeUndefined()});
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
  render(<App/>);await screen.findByRole('option',{name:'测试模型'});
  expect(screen.getByLabelText('消息')).toBeDisabled();
  fireEvent.click(screen.getByRole('button',{name:/打开工作区/}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('session_list',{workspaceId:'fresh'}));
  await waitFor(()=>expect(screen.getByLabelText('消息')).toBeEnabled());
  const input=screen.getByLabelText('消息');input.focus();expect(input).toHaveFocus();
  fireEvent.change(input,{target:{value:'梳理项目结构'}});
  expect(input).toHaveValue('梳理项目结构');
  fireEvent.click(screen.getByRole('button',{name:'发送消息'}));
  await waitFor(()=>expect(mocks.send).toHaveBeenCalledWith('first','梳理项目结构','m',expect.any(Function)));
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
  await waitFor(()=>expect(mocks.send).toHaveBeenCalledWith('other-session','给 B','m',expect.any(Function)));
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
 it('选择附件后只将宿主返回的hash随消息发送',async()=>{
  const original=mocks.command.getMockImplementation()!;mocks.command.mockImplementation(async(name,args)=>name==='artifact_import'?attachment:original(name,args));vi.mocked(open).mockResolvedValueOnce('C:/private/说明.txt');
  render(<App/>);await screen.findByText('内容 a');fireEvent.click(screen.getByRole('button',{name:'添加附件'}));await screen.findByText('说明.txt');fireEvent.change(screen.getByLabelText('消息'),{target:{value:'读取附件'}});fireEvent.click(screen.getByRole('button',{name:'发送消息'}));
  await waitFor(()=>expect(mocks.send).toHaveBeenCalledWith('a','读取附件','m',expect.any(Function),[attachment.hash]));expect(mocks.command).toHaveBeenCalledWith('artifact_import',{sessionId:'a',path:'C:/private/说明.txt'});
 });
 it('导入期间禁止发送，切换会话后丢弃迟到的附件',async()=>{
  let finish!:(value:unknown)=>void;const original=mocks.command.getMockImplementation()!;mocks.command.mockImplementation(async(name,args)=>name==='artifact_import'?new Promise(resolve=>finish=resolve):original(name,args));vi.mocked(open).mockResolvedValueOnce('C:/private/说明.txt');
  render(<App/>);await screen.findByText('内容 a');fireEvent.change(screen.getByLabelText('消息'),{target:{value:'任务'}});fireEvent.click(screen.getByRole('button',{name:'添加附件'}));await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('artifact_import',expect.anything()));expect(screen.getByRole('button',{name:'发送消息'})).toBeDisabled();fireEvent.click(screen.getByRole('button',{name:'会话 B'}));await screen.findByText('内容 b');await act(async()=>finish(attachment));expect(screen.queryByText('说明.txt')).not.toBeInTheDocument();
  fireEvent.change(screen.getByLabelText('消息'),{target:{value:'无附件'}});fireEvent.click(screen.getByRole('button',{name:'发送消息'}));await waitFor(()=>expect(mocks.send).toHaveBeenCalledWith('b','无附件','m',expect.any(Function)));
 });
 it('附件和大输出卡片通过保存对话框导出真实完整内容',async()=>{
  const original=mocks.command.getMockImplementation()!;mocks.command.mockImplementation(async(name,args)=>name==='session_events'?[{id:'attached',sessionId:'a',seq:'1',kind:'user.message',data:{content:'查看',attachments:[attachment]},createdAt:''},{id:'output',sessionId:'a',seq:'2',kind:'tool.result',data:{name:'read',content:'预览',artifact:{...attachment,name:'完整输出.txt'}},createdAt:''}]:original(name,args));vi.mocked(save).mockResolvedValueOnce('C:/saved/output.txt');
  render(<App/>);await screen.findByRole('button',{name:'下载 完整输出.txt'});fireEvent.click(screen.getByRole('button',{name:'下载 完整输出.txt'}));await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('artifact_save',{sessionId:'a',hash:attachment.hash,path:'C:/saved/output.txt'}));expect(screen.getByRole('button',{name:'下载 说明.txt'})).toBeInTheDocument();
 });
});
