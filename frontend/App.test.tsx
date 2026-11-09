import {afterEach,beforeEach,describe,expect,it,vi} from 'vitest';
import {act,cleanup,fireEvent,render,screen,waitFor,within} from '@testing-library/react';
import App from './App';
import {ChangesView} from './ChangesView';
import {GOAL_PROMPT,INIT_PROMPT,PLAN_PROMPT} from './slashCommands';
import {useChat} from './store';
import {useSidePanel} from './sidePanelStore';
import {open,save} from '@tauri-apps/plugin-dialog';
import type {LiveEvent,SessionEvent} from './types';
const mocks=vi.hoisted(()=>({command:vi.fn(),send:vi.fn()}));
vi.mock('./api',()=>({desktop:()=>true,command:mocks.command,send:mocks.send,errorText:(e:unknown)=>e instanceof Error?e.message:String(e)}));
vi.mock('@tauri-apps/plugin-dialog',()=>({open:vi.fn(),save:vi.fn()}));
const event=(id:string,sessionId:string,seq:string,content:string):SessionEvent=>({id,sessionId,seq,kind:'assistant.message',data:{content},createdAt:''});
beforeEach(()=>{useChat.setState({events:{},streams:{},statuses:{}});useSidePanel.getState().reset();mocks.command.mockReset();mocks.send.mockReset();mocks.command.mockImplementation(async(name:string,args:any)=>{
 if(name==='workspace_list')return [{id:'w',sourcePath:'D:/project',replicaPath:'D:/copy',createdAt:'',name:'project'}];
 if(name==='model_list')return [{id:'m',name:'测试模型',compatibility:'standard',providerId:'custom',apiMode:'chatCompletions',maxOutputTokens:null,baseUrl:'https://example.test',model:'m',credentialRef:'',proxy:null,efforts:['low','high'],effort:null,enabled:true,builtin:false}];
 if(name==='provider_list')return [{id:'custom',name:'自定义供应商',builtin:false,compatibility:'standard',endpoints:[{apiMode:'chatCompletions',baseUrl:'https://example.test'}],defaultModel:'m',credentialRef:'',proxy:null,enabled:true,builtinModels:[],disabledModels:[]}];
 if(name==='default_selection_get')return null;
 if(name==='session_list')return [{id:'a',workspaceId:'w',title:'会话 A'},{id:'b',workspaceId:'w',title:'会话 B'}];
 if(name==='session_events')return [event(`e-${args.sessionId}`,args.sessionId,'1',`内容 ${args.sessionId}`)];
 if(name==='approval_list')return [];
 if(name==='approval_mode_get')return 'ask';
 return [];
})});
afterEach(()=>{cleanup();vi.useRealTimers()});
describe('聊天交互',()=>{
 it('草稿发送的迟到响应不污染当前会话，会话行仍并入侧栏',async()=>{
  let resolveCreation!:(value:unknown)=>void;
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>{
   if(name==='workspace_list')return [{id:'w',sourcePath:'D:/project-A',name:'project-A'},{id:'other',sourcePath:'D:/project-B',name:'project-B'}];
   if(name==='session_create')return new Promise(resolve=>{resolveCreation=resolve});
   if(name==='session_list')return [{id:'a',workspaceId:'w',title:'会话 A'},{id:'b',workspaceId:'w',title:'会话 B'},{id:'other-session',workspaceId:'other',title:'B 的会话'}];
   return original(name,args);
  });
  render(<App/>);await screen.findByText('内容 a');
  fireEvent.click(screen.getByRole('button',{name:'新建聊天'}));
  fireEvent.change(screen.getByLabelText('消息'),{target:{value:'发给迟到会话'}});
  fireEvent.click(screen.getByRole('button',{name:'发送消息'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('session_create',{workspaceId:'w',title:'新会话'}));
  // 创建挂起期间切到 B 的会话：迟到响应不抢占 UI
  fireEvent.click(screen.getByRole('button',{name:/B 的会话/}));
  await screen.findByText('内容 other-session');
  await act(async()=>resolveCreation({id:'late-session',workspaceId:'w',title:'A 迟到的新会话'}));
  expect(screen.getByText('A 迟到的新会话')).toBeInTheDocument();
  expect(screen.getByText('内容 other-session')).toBeInTheDocument();
  // 首条消息归档到迟到会话
  await waitFor(()=>expect(mocks.send).toHaveBeenCalledWith('late-session','发给迟到会话','m',expect.any(Function),undefined,'high'));
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
 it('首段输出前显示思考状态，流式内容到达后由其接替',async()=>{
  let release!:()=>void,receive!:(event:LiveEvent)=>void;
  mocks.send.mockImplementation(async(_id:string,_text:string,_model:string,onEvent:(event:LiveEvent)=>void)=>{receive=onEvent;await new Promise<void>(resolve=>release=resolve)});
  render(<App/>);await screen.findByText('内容 a');fireEvent.change(screen.getByLabelText('消息'),{target:{value:'你好'}});fireEvent.click(screen.getByRole('button',{name:'发送消息'}));
  expect(await screen.findByRole('status',{name:'正在思考'})).toBeVisible();
  act(()=>receive({type:'delta',sessionId:'a',requestId:'r',index:0,text:'正在输出',reasoning:false}));
  expect(screen.getByText('正在输出')).toBeVisible();expect(screen.queryByRole('status',{name:'正在思考'})).not.toBeInTheDocument();
  await act(async()=>release());await waitFor(()=>expect(screen.queryByRole('status',{name:'正在思考'})).not.toBeInTheDocument());
 });
 it('当前会话等待审批时隐藏思考状态',async()=>{
  const original=mocks.command.getMockImplementation()!;let approvalPending=false,release!:()=>void;
  mocks.command.mockImplementation(async(name,args)=>name==='approval_list'&&approvalPending?[{id:'approval-1',sessionId:'a',tool:'shell',arguments:{command:'cargo test'},reason:'执行测试'}]:original(name,args));
  mocks.send.mockImplementation(async()=>new Promise<void>(resolve=>release=resolve));
  render(<App/>);await screen.findByText('内容 a');fireEvent.change(screen.getByLabelText('消息'),{target:{value:'请求'}});fireEvent.click(screen.getByRole('button',{name:'发送消息'}));
  expect(await screen.findByRole('status',{name:'正在思考'})).toBeVisible();approvalPending=true;
  await screen.findByRole('dialog',{name:'操作需要你的批准'},{timeout:3500});
  expect(screen.queryByRole('status',{name:'正在思考'})).not.toBeInTheDocument();
  await act(async()=>release());
 });
 it('取消生成后清除思考状态',async()=>{
  let release!:()=>void;mocks.send.mockImplementation(async()=>new Promise<void>(resolve=>release=resolve));
  render(<App/>);await screen.findByText('内容 a');fireEvent.change(screen.getByLabelText('消息'),{target:{value:'请求'}});fireEvent.click(screen.getByRole('button',{name:'发送消息'}));
  expect(await screen.findByRole('status',{name:'正在思考'})).toBeVisible();fireEvent.click(screen.getByRole('button',{name:'取消生成'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('session_cancel',{sessionId:'a'}));await act(async()=>release());
  await waitFor(()=>expect(screen.queryByRole('status',{name:'正在思考'})).not.toBeInTheDocument());
 });
 it('send 结束后不在历史事件回读期间重新显示思考状态',async()=>{
  let releaseSend!:()=>void,releaseRefresh!:(events:SessionEvent[])=>void,eventReads=0;
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>{
   if(name==='session_events'&&args.sessionId==='a'&&++eventReads===2)return new Promise(resolve=>releaseRefresh=resolve);
   return original(name,args);
  });
  mocks.send.mockImplementation(async()=>new Promise<void>(resolve=>releaseSend=resolve));
  render(<App/>);await screen.findByText('内容 a');fireEvent.change(screen.getByLabelText('消息'),{target:{value:'请求'}});fireEvent.click(screen.getByRole('button',{name:'发送消息'}));
  expect(await screen.findByRole('status',{name:'正在思考'})).toBeVisible();await act(async()=>releaseSend());await waitFor(()=>expect(releaseRefresh).toBeTypeOf('function'));
  const visibleWhileHistoryIsLoading=screen.queryByRole('status',{name:'正在思考'})!==null;
  await act(async()=>releaseRefresh([{id:'end',sessionId:'a',seq:'2',kind:'turn.end',data:{runId:'r',status:'completed'},createdAt:''}]));
  expect(visibleWhileHistoryIsLoading).toBe(false);expect(screen.queryByRole('status',{name:'正在思考'})).not.toBeInTheDocument();
 });
 it('发送错误可见且不插入假成功消息，也不在历史回读期间显示思考状态',async()=>{
  let rejectSend!:(reason:unknown)=>void,releaseRefresh!:(events:SessionEvent[])=>void,eventReads=0;
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>{
   if(name==='session_events'&&args.sessionId==='a'&&++eventReads===2)return new Promise(resolve=>releaseRefresh=resolve);
   return original(name,args);
  });
  mocks.send.mockImplementation(()=>new Promise<void>((_resolve,reject)=>rejectSend=reject));
  render(<App/>);await screen.findByText('内容 a');fireEvent.change(screen.getByLabelText('消息'),{target:{value:'请求'}});fireEvent.click(screen.getByRole('button',{name:'发送消息'}));
  expect(await screen.findByRole('status',{name:'正在思考'})).toBeVisible();await act(async()=>rejectSend(new Error('连接失败')));await screen.findByText('连接失败');
  await waitFor(()=>expect(releaseRefresh).toBeTypeOf('function'));
  const visibleWhileHistoryIsLoading=screen.queryByRole('status',{name:'正在思考'})!==null;
  await act(async()=>releaseRefresh([{id:'end',sessionId:'a',seq:'2',kind:'turn.end',data:{runId:'r',status:'error'},createdAt:''}]));
  expect(visibleWhileHistoryIsLoading).toBe(false);expect(screen.queryByRole('status',{name:'正在思考'})).not.toBeInTheDocument();expect(screen.getByRole('button',{name:'发送消息'})).toBeInTheDocument();
 });
 it('审批通过传递真实请求id',async()=>{const original=mocks.command.getMockImplementation()!;mocks.command.mockImplementation(async(name,args)=>name==='approval_list'?[{id:'approval-1',sessionId:'a',tool:'shell',arguments:{command:'cargo test'},reason:'执行测试'}]:original(name,args));render(<App/>);await screen.findByRole('dialog',{name:'操作需要你的批准'});fireEvent.click(screen.getByRole('button',{name:'批准此次操作'}));await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('approval_resolve',{id:'approval-1',allow:true}))});
 it('权限模式选择器加载默认档并对当前会话保存切换',async()=>{render(<App/>);await screen.findByText('内容 a');const selector=screen.getByLabelText('权限模式');expect(selector).toHaveValue('ask');fireEvent.change(selector,{target:{value:'auto'}});await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('approval_mode_save',{mode:'auto',sessionId:'a'}));expect(screen.getByLabelText('权限模式')).toHaveValue('auto')});
 it('权限模式提供五档选项',async()=>{render(<App/>);await screen.findByText('内容 a');const selector=screen.getByLabelText('权限模式');const values=Array.from(selector.querySelectorAll('option')).map(o=>o.value);expect(values).toEqual(['ask','edit','guarded','auto','never'])});
});
describe('侧栏会话入口',()=>{
 it('多项目时不再显示工作区切换下拉框，新建聊天按钮可用',async()=>{
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>{
   if(name==='workspace_list')return [{id:'w',sourcePath:'D:/A',name:'A'},{id:'other',sourcePath:'D:/B',name:'B'}];
   if(name==='session_list')return [{id:'a',workspaceId:'w',title:'会话 A'},{id:'other-session',workspaceId:'other',title:'B 的会话'}];
   return original(name,args);
  });
  render(<App/>);await screen.findByText('内容 a');
  expect(screen.queryByLabelText('工作区')).toBeNull();
  expect(screen.getByRole('button',{name:'新建聊天'})).toBeEnabled();
 });
 it('不再显示侧栏顶部的工作区选择按钮',async()=>{render(<App/>);await screen.findByText('内容 a');expect(screen.queryByText('托管工作副本')).not.toBeInTheDocument()});
 it('无工作区时欢迎页与侧栏空态提供打开工作区入口',async()=>{
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>name==='workspace_list'?[]:original(name,args));
  render(<App/>);
  await screen.findByLabelText('当前模型');
  const welcome=screen.getByText('想把什么变成现实？').closest('div')!;
  expect(within(welcome).getByRole('button',{name:/打开工作区/})).toBeEnabled();
  const nav=screen.getByText('打开工作区后，会话将保存在本机。').closest('nav')!;
  expect(within(nav).getByRole('button',{name:/打开工作区/})).toBeEnabled();
 });
 it('没有激活项目时新建聊天进入草稿态并自动弹出项目菜单',async()=>{
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>name==='workspace_list'?[]:original(name,args));
  render(<App/>);
  await screen.findByLabelText('当前模型');
  const btn=screen.getByRole('button',{name:'新建聊天'});
  expect(btn).toBeEnabled();
  fireEvent.click(btn);
  expect(await screen.findByRole('menu',{name:'选择项目'})).toBeInTheDocument();
  expect(screen.getByRole('menuitem',{name:/新建项目/})).toBeInTheDocument();
  expect(screen.getByText('想把什么变成现实？')).toBeInTheDocument();
  expect(screen.getByRole('button',{name:'发送消息'})).toBeDisabled();
 });
 it('项目组头的新建按钮为该项目新建会话并自动切换',async()=>{
  localStorage.clear();
  let createdSession:{id:string;workspaceId:string;title:string}|null=null;
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>{
   if(name==='workspace_list')return [{id:'w',sourcePath:'D:/A',name:'A'},{id:'other',sourcePath:'D:/B',name:'B'}];
   if(name==='session_list')return [...(createdSession?[createdSession]:[]),{id:'a',workspaceId:'w',title:'会话 A'},{id:'other-session',workspaceId:'other',title:'B 的会话'}];
   if(name==='session_create'){createdSession={id:'created-other',workspaceId:'other',title:'B 的新会话'};return createdSession}
   return original(name,args);
  });
  render(<App/>);await screen.findByText('内容 a');
  fireEvent.click(screen.getByRole('switch',{name:'切换视图：最近 / 项目'}));
  fireEvent.click(screen.getByRole('button',{name:'在 B 新建聊天'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('session_create',{workspaceId:'other',title:'新会话'}));
  await screen.findByText('内容 created-other');
 });
 it('当前项目组头的新建按钮直接新建会话',async()=>{
  localStorage.clear();
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>name==='session_create'?{id:'new-w',workspaceId:'w',title:'新会话'}:original(name,args));
  render(<App/>);await screen.findByText('内容 a');
  fireEvent.click(screen.getByRole('switch',{name:'切换视图：最近 / 项目'}));
  fireEvent.click(screen.getByRole('button',{name:'在 project 新建聊天'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('session_create',{workspaceId:'w',title:'新会话'}));
  await screen.findByText('内容 new-w');
 });
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
 it('edit 工具显示行级差异：调用行收敛，结果卡带统计与 diff 展开',async()=>{
  const original=mocks.command.getMockImplementation()!;
  const unified=['--- a/src/config.ts','+++ b/src/config.ts','@@ -1,2 +1,3 @@',' const a=1;','-const b=2;','+const b=3;','+const c=4;'].join('\n');
  mocks.command.mockImplementation(async(name:string,args:any)=>name==='session_events'?[
   {id:'c1',sessionId:'a',seq:'1',kind:'tool.call',data:{id:'c1',name:'edit',arguments:{path:'src/config.ts',oldText:'const b=2;',newText:'const b=3;\nconst c=4;'}},createdAt:''},
   {id:'r1',sessionId:'a',seq:'2',kind:'tool.result',data:{callId:'c1',name:'edit',content:'已修改副本 src/config.ts',isError:false,status:'completed',diff:unified,path:'src/config.ts'},createdAt:''},
  ]:original(name,args));
  render(<App/>);
  expect((await screen.findAllByText('编辑')).length).toBe(2);
  expect(screen.getAllByRole('button',{name:'config.ts'})).toHaveLength(2);
  expect(screen.queryByText('调用中')).not.toBeInTheDocument();
  expect(screen.getAllByText('src')).toHaveLength(2);
  expect(screen.getByText('+2')).toBeInTheDocument();
  expect(screen.getByText('-1')).toBeInTheDocument();
  fireEvent.click(screen.getByText('已完成'));
  expect(await screen.findByText('+const b=3;')).toBeVisible();
  expect(screen.queryByText(/oldText/)).not.toBeInTheDocument();
 });
 it('edit 失败结果无 diff，展开显示错误内容',async()=>{
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name:string,args:any)=>name==='session_events'?[
   {id:'r1',sessionId:'a',seq:'1',kind:'tool.result',data:{callId:'c1',name:'edit',content:'待替换文本必须在当前文件中恰好出现一次',isError:true,status:'error',diff:null,path:'src/config.ts'},createdAt:''},
  ]:original(name,args));
  render(<App/>);
  expect(await screen.findByText('执行失败')).toBeInTheDocument();
  expect(screen.queryByText(/^\+\d+$/)).not.toBeInTheDocument();
  fireEvent.click(screen.getByText('执行失败'));
  expect(await screen.findByText(/恰好出现一次/)).toBeVisible();
 });
 it('write 工具结果显示写入标签与全新增 diff',async()=>{
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name:string,args:any)=>name==='session_events'?[
   {id:'r1',sessionId:'a',seq:'1',kind:'tool.result',data:{callId:'c1',name:'write',content:'已写入副本 docs/new.md',isError:false,status:'completed',diff:'--- /dev/null\n+++ b/docs/new.md\n@@ -0,0 +1,1 @@\n+# 标题',path:'docs/new.md'},createdAt:''},
  ]:original(name,args));
  render(<App/>);
  expect(await screen.findByText('写入')).toBeInTheDocument();
  expect(screen.getByText('+1')).toBeInTheDocument();
  fireEvent.click(screen.getByText('已完成'));
  expect(await screen.findByText('+# 标题')).toBeVisible();
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
 it('选择没有会话的新工作区后进入草稿态，发送首条消息时创建会话',async()=>{
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>{
   if(name==='workspace_list')return [];
   if(name==='workspace_open')return {id:'fresh',sourcePath:'D:/fresh',replicaPath:'D:/copy-fresh',createdAt:'',name:'fresh',pinned:false};
   if(name==='session_list')return [];
   if(name==='session_create')return {id:'first',workspaceId:'fresh',title:'新会话'};
   if(name==='session_events')return [];
   return original(name,args);
  });
  vi.mocked(open).mockResolvedValueOnce('D:/fresh');
  render(<App/>);await screen.findByLabelText('当前模型');
  const welcome=screen.getByText('想把什么变成现实？').closest('div')!;
  fireEvent.click(within(welcome).getByRole('button',{name:/打开工作区/}));
  // 命名弹窗预填目录名，确认后携带名称创建
  const dialog=await screen.findByRole('dialog',{name:'打开工作区'});
  expect(screen.getByLabelText('项目名称')).toHaveValue('fresh');
  fireEvent.click(within(dialog).getByRole('button',{name:'创建'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('workspace_open',{path:'D:/fresh',name:'fresh',executionMode:'replica'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('session_list',{workspaceId:'fresh',allWorkspaces:true}));
  // 创建后进入草稿态：不自动创建会话，欢迎语带项目名，可直接输入
  await waitFor(()=>expect(screen.getByRole('heading',{name:/你想在 fresh 中构建什么/})).toBeInTheDocument());
  expect(mocks.command.mock.calls.filter(([name])=>name==='session_create')).toHaveLength(0);
  const input=screen.getByLabelText('消息');
  fireEvent.change(input,{target:{value:'梳理项目结构'}});
  expect(input).toHaveValue('梳理项目结构');
  fireEvent.click(screen.getByRole('button',{name:'发送消息'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('session_create',{workspaceId:'fresh',title:'新会话'}));
  await waitFor(()=>expect(mocks.send).toHaveBeenCalledWith('first','梳理项目结构','m',expect.any(Function),undefined,'high'));
 });
 it('启动时无会话的工作区进入草稿态，发送首条消息时才创建',async()=>{
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>{
   if(name==='session_list')return [];
   if(name==='session_create')return {id:'restored',workspaceId:'w',title:'新会话'};
   if(name==='session_events')return [];
   return original(name,args);
  });
  render(<App/>);
  await screen.findByRole('heading',{name:/你想在 project 中构建什么/});
  expect(mocks.command.mock.calls.filter(([name])=>name==='session_create')).toHaveLength(0);
  const input=screen.getByLabelText('消息');
  fireEvent.change(input,{target:{value:'开始干活'}});
  fireEvent.click(screen.getByRole('button',{name:'发送消息'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('session_create',{workspaceId:'w',title:'新会话'}));
  await waitFor(()=>expect(mocks.send).toHaveBeenCalledWith('restored','开始干活','m',expect.any(Function),undefined,'high'));
 });
 it('草稿态发送时创建会话失败会显示错误，再次发送可重试',async()=>{
  let fail=true;
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>{
   if(name==='session_list')return [];
   if(name==='session_create'){if(fail)throw new Error('创建会话失败');return {id:'retry',workspaceId:'w',title:'新会话'};}
   if(name==='session_events')return [];
   return original(name,args);
  });
  render(<App/>);await screen.findByRole('heading',{name:/你想在 project 中构建什么/});
  fireEvent.change(screen.getByLabelText('消息'),{target:{value:'第一次'}});
  fireEvent.click(screen.getByRole('button',{name:'发送消息'}));
  await screen.findByText('创建会话失败');
  fail=false;
  fireEvent.change(screen.getByLabelText('消息'),{target:{value:'第二次'}});
  fireEvent.click(screen.getByRole('button',{name:'发送消息'}));
  await waitFor(()=>expect(mocks.send).toHaveBeenCalledWith('retry','第二次','m',expect.any(Function),undefined,'high'));
 });
 it('草稿态欢迎语带项目名，chip 菜单可搜索过滤项目',async()=>{
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>{
   if(name==='workspace_list')return [{id:'w',sourcePath:'D:/A',replicaPath:'D:/ca',createdAt:'',name:'project',pinned:false},{id:'other',sourcePath:'D:/B',replicaPath:'D:/cb',createdAt:'',name:'其他项目',pinned:false}];
   if(name==='session_list')return [{id:'a',workspaceId:'w',title:'会话 A',createdAt:'',updatedAt:new Date().toISOString(),parentId:null}];
   return original(name,args);
  });
  render(<App/>);await screen.findByText('内容 a');
  fireEvent.click(screen.getByRole('button',{name:'新建聊天'}));
  expect(screen.getByRole('heading',{name:/你想在 project 中构建什么/})).toBeInTheDocument();
  const menu=screen.getByRole('menu',{name:'选择项目'});
  expect(within(menu).getByRole('menuitem',{name:/其他项目/})).toBeInTheDocument();
  fireEvent.change(within(menu).getByLabelText('搜索项目'),{target:{value:'其他'}});
  expect(within(menu).getByRole('menuitem',{name:/其他项目/})).toBeInTheDocument();
  expect(within(menu).queryByRole('menuitem',{name:/project/})).not.toBeInTheDocument();
  fireEvent.change(within(menu).getByLabelText('搜索项目'),{target:{value:'zzz'}});
  expect(screen.getByText('没有匹配的项目')).toBeInTheDocument();
 });
 it('草稿态切换项目后发送在目标项目创建会话并切换工作区',async()=>{
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>{
   if(name==='workspace_list')return [{id:'w',sourcePath:'D:/A',replicaPath:'D:/ca',createdAt:'',name:'project',pinned:false},{id:'other',sourcePath:'D:/B',replicaPath:'D:/cb',createdAt:'',name:'other-pj',pinned:false}];
   if(name==='session_list')return [{id:'a',workspaceId:'w',title:'会话 A',createdAt:'',updatedAt:new Date().toISOString(),parentId:null}];
   if(name==='session_create')return {id:'in-other',workspaceId:'other',title:'新会话'};
   if(name==='session_events')return [];
   return original(name,args);
  });
  render(<App/>);await screen.findByLabelText('当前模型');
  fireEvent.click(screen.getByRole('button',{name:'新建聊天'}));
  const menu=screen.getByRole('menu',{name:'选择项目'});
  fireEvent.click(within(menu).getByRole('menuitem',{name:/other-pj/}));
  expect(screen.getByRole('heading',{name:/你想在 other-pj 中构建什么/})).toBeInTheDocument();
  fireEvent.change(screen.getByLabelText('消息'),{target:{value:'在别的项目开始'}});
  fireEvent.click(screen.getByRole('button',{name:'发送消息'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('session_create',{workspaceId:'other',title:'新会话'}));
  await waitFor(()=>expect(mocks.send).toHaveBeenCalledWith('in-other','在别的项目开始','m',expect.any(Function),undefined,'high'));
  await waitFor(()=>expect(screen.getAllByText('other-pj').length).toBeGreaterThanOrEqual(1));
 });
 it('草稿态菜单的新建项目入口走目录选择与命名弹窗',async()=>{
  vi.mocked(open).mockResolvedValueOnce('D:/newdir');
  render(<App/>);await screen.findByText('内容 a');
  fireEvent.click(screen.getByRole('button',{name:'新建聊天'}));
  const menu=screen.getByRole('menu',{name:'选择项目'});
  fireEvent.click(within(menu).getByRole('menuitem',{name:/新建项目/}));
  const dialog=await screen.findByRole('dialog',{name:'打开工作区'});
  expect(screen.getByLabelText('项目名称')).toHaveValue('newdir');
 });
 it('草稿首会话的迟到响应不能覆盖已切换到的会话，会话行仍并入侧栏',async()=>{
  let resolveCreation!:(value:unknown)=>void;
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>{
   if(name==='workspace_list')return [{id:'w',sourcePath:'D:/A',name:'A'},{id:'other',sourcePath:'D:/B',name:'B'}];
   if(name==='session_list')return [{id:'other-session',workspaceId:'other',title:'B 的会话'}];
   if(name==='session_create')return new Promise(resolve=>{resolveCreation=resolve});
   return original(name,args);
  });
  render(<App/>);
  await screen.findByRole('heading',{name:/你想在 A 中构建什么/});
  fireEvent.change(screen.getByLabelText('消息'),{target:{value:'首个任务'}});
  fireEvent.click(screen.getByRole('button',{name:'发送消息'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('session_create',{workspaceId:'w',title:'新会话'}));
  fireEvent.click(screen.getByRole('button',{name:/B 的会话/}));
  await screen.findByText('内容 other-session');
  await act(async()=>resolveCreation({id:'late',workspaceId:'w',title:'A 迟到的会话'}));
  expect(screen.getByText('A 迟到的会话')).toBeInTheDocument();
  expect(screen.getByLabelText('消息')).toBeEnabled();
  expect(screen.getByText('内容 other-session')).toBeInTheDocument();
 });
 it('组头新建挂起期间重复点击复用同一创建请求',async()=>{
  let resolveCreation!:(value:unknown)=>void;
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>{
   if(name==='workspace_list')return [{id:'other',sourcePath:'D:/B',name:'B'},{id:'w',sourcePath:'D:/A',name:'A'}];
   if(name==='session_list')return [{id:'b1',workspaceId:'other',title:'B 的旧会话'}];
   if(name==='session_create')return new Promise(resolve=>{resolveCreation=resolve});
   return original(name,args);
  });
  localStorage.clear();
  render(<App/>);
  await screen.findByText('内容 b1');
  fireEvent.click(screen.getByRole('switch',{name:'切换视图：最近 / 项目'}));
  fireEvent.click(screen.getByRole('button',{name:'在 B 新建聊天'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('session_create',{workspaceId:'other',title:'新会话'}));
  // 挂起期间重复点击：复用同一创建请求，不重复调 session_create
  fireEvent.click(screen.getByRole('button',{name:'在 B 新建聊天'}));
  await act(async()=>resolveCreation({id:'b2',workspaceId:'other',title:'B 的新会话'}));
  expect(mocks.command.mock.calls.filter(([name])=>name==='session_create')).toHaveLength(1);
  await waitFor(()=>expect(screen.getAllByText('B 的新会话').length).toBeGreaterThanOrEqual(1));
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
 it('Enter 执行选中命令：/clear 回到草稿态',async()=>{
  render(<App/>);await screen.findByText('内容 a');
  const input=screen.getByLabelText('消息');
  fireEvent.change(input,{target:{value:'/cl'}});
  fireEvent.keyDown(input,{key:'Enter'});
  // /clear 进入草稿态：欢迎语带当前项目名，不立即创建会话
  await screen.findByRole('heading',{name:/你想在 project 中构建什么/});
  expect(mocks.command.mock.calls.filter(([name])=>name==='session_create')).toHaveLength(0);
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

describe('补全弹层滚动跟随',()=>{
 // jsdom 未实现 scrollIntoView：测试期间替换为 spy，结束后还原。
 let scrollIntoView:ReturnType<typeof vi.fn>;
 beforeEach(()=>{scrollIntoView=vi.fn();(Element.prototype as any).scrollIntoView=scrollIntoView});
 afterEach(()=>{delete (Element.prototype as any).scrollIntoView});
 it('斜杠菜单方向键移动选中项时列表滚动跟随',async()=>{
  render(<App/>);await screen.findByText('内容 a');
  const input=screen.getByLabelText('消息');
  fireEvent.change(input,{target:{value:'/'}});
  fireEvent.keyDown(input,{key:'ArrowDown'});
  expect(scrollIntoView).toHaveBeenCalled();
  expect(scrollIntoView).toHaveBeenLastCalledWith({block:'nearest'});
 });
 it('技能菜单键盘选择滚动跟随，Tab 补全选中项为规范名',async()=>{
  const original=mocks.command.getMockImplementation()!;
  const skills=Array.from({length:20},(_,i)=>({name:`skill-${i}`,description:`技能描述 ${i}`,path:`D:/skills/skill-${i}`,scope:'workspace',enabled:true,brokenReason:null}));
  mocks.command.mockImplementation(async(name:string,args:any)=>name==='skill_list'?skills:original(name,args));
  render(<App/>);await screen.findByText('内容 a');
  const input=screen.getByLabelText('消息');
  fireEvent.change(input,{target:{value:'$'}});
  const options=within(screen.getByRole('listbox',{name:'技能补全'})).getAllByRole('option');
  expect(options).toHaveLength(20);
  expect(options[0]).toHaveAttribute('aria-selected','true');
  fireEvent.keyDown(input,{key:'ArrowDown'});
  expect(options[1]).toHaveAttribute('aria-selected','true');
  expect(scrollIntoView).toHaveBeenCalled();
  expect(scrollIntoView).toHaveBeenLastCalledWith({block:'nearest'});
  fireEvent.keyDown(input,{key:'Tab'});
  expect(input).toHaveValue('$skill-1 ');
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
  fireEvent.mouseDown(screen.getByRole('button',{name:'新建聊天'}));
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
    {id:'w',sourcePath:'D:/alpha',replicaPath:'D:/copy-a',createdAt:'',name:'alpha',pinned:false},
    {id:'other',sourcePath:'D:/beta',replicaPath:'D:/copy-b',createdAt:'',name:'beta',pinned:false}];
   if(name==='session_list')return [
    {id:'a',workspaceId:'w',title:'会话 A',createdAt:'',updatedAt:new Date().toISOString(),parentId:null},
    {id:'b',workspaceId:'other',title:'B 的会话',createdAt:'',updatedAt:new Date().toISOString(),parentId:null}];
   if(name==='workspace_set_pinned')return {id:args.workspaceId,sourcePath:'D:/alpha',replicaPath:'D:/copy-a',createdAt:'',name:'alpha',pinned:args.pinned};
   if(name==='session_events')return [event(`e-${args.sessionId}`,args.sessionId,'1',`内容 ${args.sessionId}`)];
   return [];
  });
 }
 it('「最近」视图按时间分组并展示跨工作区会话，switch 切到「项目」按工作区分组',async()=>{
  localStorage.clear();installSessionMocks();
  render(<App/>);await screen.findByText('内容 a');
  // 回归：switch 外不应再有「最近」文本（spec：标题行右侧仅 switch 内嵌两词）
  for(const el of screen.getAllByText('最近')) expect(el.closest('.view-switch')).not.toBeNull();
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
   if(name==='workspace_list')return [{id:'w',sourcePath:'D:/alpha',replicaPath:'D:/copy-a',createdAt:'',name:'alpha',pinned:false}];
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

describe('项目命名',()=>{
 function namingBackend(options:{empty?:boolean;renameError?:string}={}){
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>{
   if(name==='workspace_list')return options.empty?[]:[{id:'w',sourcePath:'D:/alpha',replicaPath:'D:/copy-a',createdAt:'',name:'alpha'}];
   if(name==='workspace_rename'){if(options.renameError)throw new Error(options.renameError);return {id:args.workspaceId,sourcePath:'D:/alpha',replicaPath:'D:/copy-a',createdAt:'',name:args.name}}
   if(name==='workspace_open')return {id:'fresh',sourcePath:args.path,replicaPath:'D:/copy',createdAt:'',name:args.name||'alpha-dir'};
   if(name==='session_list')return [{id:'a',workspaceId:'w',title:'会话 A',createdAt:'',updatedAt:new Date().toISOString(),parentId:null}];
   if(name==='session_events')return [event(`e-${args.sessionId}`,args.sessionId,'1',`内容 ${args.sessionId}`)];
   return original(name,args);
  });
 }
 it('打开工作区时名称留空则回退目录名',async()=>{
  localStorage.clear();namingBackend({empty:true});
  vi.mocked(open).mockResolvedValueOnce('D:/alpha-dir');
  render(<App/>);await screen.findByLabelText('当前模型');
  const welcome=screen.getByText('想把什么变成现实？').closest('div')!;
  fireEvent.click(within(welcome).getByRole('button',{name:/打开工作区/}));
  await screen.findByRole('dialog',{name:'打开工作区'});
  expect(screen.getByText('留空则使用目录名「alpha-dir」。')).toBeInTheDocument();
  fireEvent.change(screen.getByLabelText('项目名称'),{target:{value:'  '}});
  fireEvent.click(screen.getByRole('button',{name:'创建'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('workspace_open',{path:'D:/alpha-dir',name:null,executionMode:'replica'}));
  // 后端归一化后的目录名显示在面包屑上
  await waitFor(()=>expect(screen.getAllByText('alpha-dir').length).toBeGreaterThanOrEqual(1));
 });
 it('项目组头可重命名并立即显示新名称',async()=>{
  localStorage.clear();namingBackend();
  render(<App/>);await screen.findByText('内容 a');
  fireEvent.click(screen.getByRole('switch',{name:'切换视图：最近 / 项目'}));
  expect(screen.getByRole('button',{name:/^alpha \d+$/})).toBeInTheDocument();
  fireEvent.click(screen.getByRole('button',{name:'重命名 alpha'}));
  const dialog=await screen.findByRole('dialog',{name:'重命名项目'});
  expect(screen.getByLabelText('项目名称')).toHaveValue('alpha');
  fireEvent.change(screen.getByLabelText('项目名称'),{target:{value:'新名字'}});
  fireEvent.click(within(dialog).getByRole('button',{name:'保存'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('workspace_rename',{workspaceId:'w',name:'新名字'}));
  expect(screen.getByRole('button',{name:/^新名字 \d+$/})).toBeInTheDocument();
  expect(screen.queryByRole('button',{name:/^alpha \d+$/})).not.toBeInTheDocument();
 });
 it('重命名留空提交时显示后端错误且名称不变',async()=>{
  localStorage.clear();namingBackend({renameError:'名称不能为空'});
  render(<App/>);await screen.findByText('内容 a');
  fireEvent.click(screen.getByRole('switch',{name:'切换视图：最近 / 项目'}));
  fireEvent.click(screen.getByRole('button',{name:'重命名 alpha'}));
  await screen.findByRole('dialog',{name:'重命名项目'});
  fireEvent.change(screen.getByLabelText('项目名称'),{target:{value:'  '}});
  fireEvent.click(screen.getByRole('button',{name:'保存'}));
  await screen.findByText('名称不能为空');
  expect(screen.getByRole('button',{name:/^alpha \d+$/})).toBeInTheDocument();
 });
 it('中文与空格路径预填目录名并随创建提交',async()=>{
  localStorage.clear();namingBackend({empty:true});
  vi.mocked(open).mockResolvedValueOnce('D:/我的 项目');
  render(<App/>);await screen.findByLabelText('当前模型');
  const welcome=screen.getByText('想把什么变成现实？').closest('div')!;
  fireEvent.click(within(welcome).getByRole('button',{name:/打开工作区/}));
  await screen.findByRole('dialog',{name:'打开工作区'});
  // 预填保留空格与中文；确认后原样携带名称提交
  expect(screen.getByLabelText('项目名称')).toHaveValue('我的 项目');
  fireEvent.click(screen.getByRole('button',{name:'创建'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('workspace_open',{path:'D:/我的 项目',name:'我的 项目',executionMode:'replica'}));
  // 后端返回的中文名称显示在面包屑上
  await waitFor(()=>expect(screen.getAllByText('我的 项目').length).toBeGreaterThanOrEqual(1));
 });
});

describe('会话命名',()=>{
 beforeEach(()=>localStorage.clear());
 it('重命名旧会话后仍按更新时间排序',async()=>{
  const newer={id:'newer',workspaceId:'w',title:'较新的会话',createdAt:'',updatedAt:'2026-11-09T11:00:00Z',parentId:null};
  const older={id:'older',workspaceId:'w',title:'较旧的会话',createdAt:'',updatedAt:'2026-11-08T11:00:00Z',parentId:null};
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>{
   if(name==='workspace_list')return [{id:'w',sourcePath:'D:/alpha',replicaPath:'D:/copy-a',createdAt:'',name:'alpha'}];
   if(name==='session_list')return [newer,older];
   if(name==='session_events')return args.sessionId==='newer'?[event('e-newer','newer','1','新会话内容')]:[];
   if(name==='session_rename')return {...older,title:args.title};
   return original(name,args);
  });
  render(<App/>);await screen.findByText('新会话内容');
  const sessionRows=()=>screen.getAllByRole('button').filter(button=>button.classList.contains('session-row'));
  expect(sessionRows().map(button=>button.querySelector('.session-title')?.textContent)).toEqual(['较新的会话','较旧的会话']);
  fireEvent.contextMenu(screen.getByRole('button',{name:/较旧的会话/}));
  fireEvent.click(await screen.findByRole('menuitem',{name:'重命名'}));
  const dialog=await screen.findByRole('dialog',{name:'重命名会话'});
  fireEvent.change(screen.getByLabelText('会话名称'),{target:{value:'已重命名的旧会话'}});
  fireEvent.click(within(dialog).getByRole('button',{name:'保存'}));
  await screen.findByText('已重命名的旧会话');
  expect(sessionRows().map(button=>button.querySelector('.session-title')?.textContent)).toEqual(['较新的会话','已重命名的旧会话']);
 });
 it('首问后的标题响应按新的更新时间重新排序',async()=>{
  const newer={id:'newer',workspaceId:'w',title:'较新的会话',createdAt:'',updatedAt:'2026-11-09T11:00:00Z',parentId:null};
  const older={id:'older',workspaceId:'w',title:'新会话',createdAt:'',updatedAt:'2026-11-08T11:00:00Z',parentId:null};
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>{
   if(name==='session_list')return [newer,older];
   if(name==='session_events')return args.sessionId==='newer'?[event('e-newer','newer','1','较新会话内容')]:[];
   if(name==='session_title_generate')return {...older,title:'首问自动标题',updatedAt:'2026-11-09T12:00:00Z'};
   return original(name,args);
  });
  render(<App/>);await screen.findByText('较新会话内容');
  fireEvent.click(screen.getByRole('button',{name:/新会话/}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('session_events',{sessionId:'older'}));
  fireEvent.change(screen.getByLabelText('消息'),{target:{value:'今天首次提问'}});
  fireEvent.click(screen.getByRole('button',{name:'发送消息'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('session_title_generate',{sessionId:'older',text:'今天首次提问',modelId:'m',effort:'high',attachmentNames:[]}));
  expect(await screen.findAllByText('首问自动标题')).toHaveLength(2);
  const titles=screen.getAllByRole('button').filter(button=>button.classList.contains('session-row')).map(button=>button.querySelector('.session-title')?.textContent);
  expect(titles).toEqual(['首问自动标题','较新的会话']);
 });
 it('重命名子代理会话后保留子任务标记',async()=>{
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>{
   if(name==='session_list')return [
    {id:'parent',workspaceId:'w',title:'父会话',createdAt:'',updatedAt:'2026-11-09T11:00:00Z',parentId:null},
    {id:'child',workspaceId:'w',title:'子会话',createdAt:'',updatedAt:'2026-11-09T10:00:00Z',parentId:'parent'}];
   if(name==='session_events')return args.sessionId==='parent'?[event('e-parent','parent','1','父消息')]:[];
   if(name==='session_rename')return {id:'child',workspaceId:'w',title:args.title,createdAt:'',updatedAt:'2026-11-09T10:00:00Z',parentId:null};
   return original(name,args);
  });
  render(<App/>);await screen.findByText('父消息');
  fireEvent.contextMenu(screen.getByRole('button',{name:/子会话/}));
  fireEvent.click(await screen.findByRole('menuitem',{name:'重命名'}));
  const dialog=await screen.findByRole('dialog',{name:'重命名会话'});
  fireEvent.change(screen.getByLabelText('会话名称'),{target:{value:'已改名子会话'}});
  fireEvent.click(within(dialog).getByRole('button',{name:'保存'}));
  expect(await screen.findByText('已改名子会话')).toBeInTheDocument();
  expect(screen.getByText('子任务')).toBeInTheDocument();
 });
 it.each(['最近','项目'])('在%s视图右键会话可重命名并更新列表和面包屑',async view=>{
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>name==='session_rename'?{id:args.sessionId,workspaceId:'w',title:args.title,createdAt:'',updatedAt:''}:original(name,args));
  render(<App/>);await screen.findByText('内容 a');
  if(view==='项目')fireEvent.click(screen.getByRole('switch',{name:'切换视图：最近 / 项目'}));
  fireEvent.contextMenu(screen.getByRole('button',{name:/会话 A/}));
  fireEvent.click(await screen.findByRole('menuitem',{name:'重命名'}));
  const dialog=await screen.findByRole('dialog',{name:'重命名会话'});
  expect(screen.getByLabelText('会话名称')).toHaveValue('会话 A');
  fireEvent.change(screen.getByLabelText('会话名称'),{target:{value:'重命名后的会话'}});
  fireEvent.click(within(dialog).getByRole('button',{name:'保存'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('session_rename',{sessionId:'a',title:'重命名后的会话'}));
  expect(await screen.findAllByText('重命名后的会话')).toHaveLength(2);
  expect(screen.queryByRole('menu',{name:'会话操作'})).not.toBeInTheDocument();
 });
 it('首问创建草稿会话时并行生成标题和发送消息',async()=>{
  let resolveTitle!:(value:unknown)=>void;
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>{
   if(name==='session_list')return [];
   if(name==='session_create')return {id:'first',workspaceId:'w',title:'新会话',createdAt:'',updatedAt:''};
   if(name==='session_title_generate')return new Promise(resolve=>{resolveTitle=resolve});
   if(name==='session_events')return [];
   return original(name,args);
  });
  render(<App/>);await screen.findByRole('button',{name:'选择项目'});
  fireEvent.change(screen.getByLabelText('消息'),{target:{value:'修复登录状态刷新后丢失'}});
  await waitFor(()=>expect(screen.getByRole('button',{name:'发送消息'})).toBeEnabled());
  fireEvent.click(screen.getByRole('button',{name:'发送消息'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('session_title_generate',{sessionId:'first',text:'修复登录状态刷新后丢失',modelId:'m',effort:'high',attachmentNames:[]}));
  expect(mocks.send).toHaveBeenCalledWith('first','修复登录状态刷新后丢失','m',expect.any(Function),undefined,'high');
  await act(async()=>resolveTitle({id:'first',workspaceId:'w',title:'修复登录状态丢失',createdAt:'',updatedAt:''}));
  expect(await screen.findAllByText('修复登录状态丢失')).toHaveLength(2);
 });
 it('标题命令失败不会阻止用户首问发送',async()=>{
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>{
   if(name==='session_list')return [];
   if(name==='session_create')return {id:'first',workspaceId:'w',title:'新会话',createdAt:'',updatedAt:''};
   if(name==='session_title_generate')throw new Error('标题服务不可用');
   if(name==='session_events')return [];
   return original(name,args);
  });
  render(<App/>);await screen.findByRole('button',{name:'选择项目'});
  fireEvent.change(screen.getByLabelText('消息'),{target:{value:'检查构建错误'}});
  fireEvent.click(screen.getByRole('button',{name:'发送消息'}));
  await waitFor(()=>expect(mocks.send).toHaveBeenCalledWith('first','检查构建错误','m',expect.any(Function),undefined,'high'));
  expect(screen.queryByRole('alert')).not.toBeInTheDocument();
 });
 it('已有空会话的首问也触发标题生成',async()=>{
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>{
   if(name==='session_list')return [{id:'blank',workspaceId:'w',title:'新会话',createdAt:'',updatedAt:''}];
   if(name==='session_events'&&args.sessionId==='blank')return [];
   if(name==='session_title_generate')return {id:'blank',workspaceId:'w',title:'空会话首问',createdAt:'',updatedAt:''};
   return original(name,args);
  });
  render(<App/>);await screen.findByRole('button',{name:/新会话/});
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('session_events',{sessionId:'blank'}));
  fireEvent.change(screen.getByLabelText('消息'),{target:{value:'梳理项目入口'}});
  fireEvent.click(screen.getByRole('button',{name:'发送消息'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('session_title_generate',{sessionId:'blank',text:'梳理项目入口',modelId:'m',effort:'high',attachmentNames:[]}));
  expect(await screen.findAllByText('空会话首问')).toHaveLength(2);
 });
 it('空会话的首条 /init 也并行生成标题',async()=>{
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>{
   if(name==='session_list')return [{id:'blank',workspaceId:'w',title:'新会话',createdAt:'',updatedAt:'',parentId:null}];
   if(name==='session_events'&&args.sessionId==='blank')return [];
   if(name==='session_title_generate')return {id:'blank',workspaceId:'w',title:'初始化项目说明',createdAt:'',updatedAt:''};
   return original(name,args);
  });
  render(<App/>);await screen.findByRole('button',{name:/新会话/});
  const input=screen.getByLabelText('消息');
  fireEvent.change(input,{target:{value:'/init'}});
  fireEvent.keyDown(input,{key:'Enter'});
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('session_title_generate',{sessionId:'blank',text:INIT_PROMPT,modelId:'m',effort:'high',attachmentNames:[]}));
  expect(mocks.send).toHaveBeenCalledWith('blank',INIT_PROMPT,'m',expect.any(Function),undefined,'high');
  expect(await screen.findAllByText('初始化项目说明')).toHaveLength(2);
 });
 it('迟到的自动标题响应不会覆盖人工重命名',async()=>{
  let resolveTitle!:(value:unknown)=>void;
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>{
   if(name==='session_list')return [{id:'blank',workspaceId:'w',title:'新会话',createdAt:'',updatedAt:'',parentId:null}];
   if(name==='session_events'&&args.sessionId==='blank')return [];
   if(name==='session_title_generate')return new Promise(resolve=>{resolveTitle=resolve});
   if(name==='session_rename')return {id:'blank',workspaceId:'w',title:args.title,createdAt:'',updatedAt:'',parentId:null};
   return original(name,args);
  });
  render(<App/>);await screen.findByRole('button',{name:/新会话/});
  fireEvent.change(screen.getByLabelText('消息'),{target:{value:'首个问题'}});
  fireEvent.click(screen.getByRole('button',{name:'发送消息'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('session_title_generate',{sessionId:'blank',text:'首个问题',modelId:'m',effort:'high',attachmentNames:[]}));
  fireEvent.contextMenu(screen.getByRole('button',{name:/新会话/}));
  fireEvent.click(await screen.findByRole('menuitem',{name:'重命名'}));
  const dialog=await screen.findByRole('dialog',{name:'重命名会话'});
  fireEvent.change(screen.getByLabelText('会话名称'),{target:{value:'人工标题'}});
  fireEvent.click(within(dialog).getByRole('button',{name:'保存'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('session_rename',{sessionId:'blank',title:'人工标题'}));
  await act(async()=>resolveTitle({id:'blank',workspaceId:'w',title:'自动生成标题',createdAt:'',updatedAt:'',parentId:null}));
  expect(await screen.findAllByText('人工标题')).toHaveLength(2);
  expect(screen.queryByText('自动生成标题')).not.toBeInTheDocument();
 });
 it('迟到的会话列表快照不会覆盖已生成标题',async()=>{
  let resolveTitle!:(value:unknown)=>void,resolveList!:(value:unknown)=>void;
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>{
   if(name==='workspace_list')return [
    {id:'w',sourcePath:'D:/alpha',replicaPath:'D:/copy-a',createdAt:'',name:'alpha'},
    {id:'other',sourcePath:'D:/beta',replicaPath:'D:/copy-b',createdAt:'',name:'beta'}];
   if(name==='session_list'&&args.workspaceId==='w')return [
    {id:'blank',workspaceId:'w',title:'新会话',createdAt:'',updatedAt:'2026-11-09T11:00:00Z',parentId:null},
    {id:'other-session',workspaceId:'other',title:'B 的会话',createdAt:'',updatedAt:'2026-11-09T10:00:00Z',parentId:null}];
   if(name==='session_list'&&args.workspaceId==='other')return new Promise(resolve=>{resolveList=resolve});
   if(name==='session_events'&&args.sessionId==='blank')return [];
   if(name==='session_events'&&args.sessionId==='other-session')return [event('e-other','other-session','1','B 的内容')];
   if(name==='session_title_generate')return new Promise(resolve=>{resolveTitle=resolve});
   return original(name,args);
  });
  render(<App/>);await screen.findByRole('button',{name:/新会话/});
  fireEvent.change(screen.getByLabelText('消息'),{target:{value:'首个问题'}});
  fireEvent.click(screen.getByRole('button',{name:'发送消息'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('session_title_generate',{sessionId:'blank',text:'首个问题',modelId:'m',effort:'high',attachmentNames:[]}));
  fireEvent.click(screen.getByRole('button',{name:/B 的会话/}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('session_list',{workspaceId:'other',allWorkspaces:true}));
  await act(async()=>resolveTitle({id:'blank',workspaceId:'w',title:'生成完成标题',createdAt:'',updatedAt:'2026-11-09T11:00:00Z',parentId:null}));
  expect(await screen.findByText('生成完成标题')).toBeInTheDocument();
  await act(async()=>resolveList([
   {id:'blank',workspaceId:'w',title:'新会话',createdAt:'',updatedAt:'2026-11-09T11:00:00Z',parentId:null},
   {id:'other-session',workspaceId:'other',title:'B 的会话',createdAt:'',updatedAt:'2026-11-09T10:00:00Z',parentId:null}]));
  expect(screen.getByText('生成完成标题')).toBeInTheDocument();
 });
 it('有历史用户消息的会话不再生成首次标题',async()=>{
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>{
   if(name==='session_list')return [{id:'used',workspaceId:'w',title:'新会话',createdAt:'',updatedAt:''}];
   if(name==='session_events'&&args.sessionId==='used')return [{id:'old-user',sessionId:'used',seq:'1',kind:'user.message',data:{content:'之前的问题'},createdAt:''}];
   return original(name,args);
  });
  render(<App/>);await screen.findByText('之前的问题');
  await waitFor(()=>expect(useChat.getState().events.used?.[0]?.kind).toBe('user.message'));
  fireEvent.change(screen.getByLabelText('消息'),{target:{value:'后续问题'}});
  fireEvent.click(screen.getByRole('button',{name:'发送消息'}));
  await waitFor(()=>expect(mocks.send).toHaveBeenCalledWith('used','后续问题','m',expect.any(Function),undefined,'high'));
  expect(mocks.command).not.toHaveBeenCalledWith('session_title_generate',expect.anything());
 });
 it('会话右键菜单可用 Escape 和点击外部关闭',async()=>{
  render(<App/>);await screen.findByText('内容 a');
  fireEvent.contextMenu(screen.getByRole('button',{name:/会话 A/}));
  expect(await screen.findByRole('menu',{name:'会话操作'})).toBeInTheDocument();
  fireEvent.keyDown(document,{key:'Escape'});
  await waitFor(()=>expect(screen.queryByRole('menu',{name:'会话操作'})).not.toBeInTheDocument());
  fireEvent.contextMenu(screen.getByRole('button',{name:/会话 A/}));
  expect(await screen.findByRole('menu',{name:'会话操作'})).toBeInTheDocument();
  fireEvent.mouseDown(document.body);
  await waitFor(()=>expect(screen.queryByRole('menu',{name:'会话操作'})).not.toBeInTheDocument());
 });
 it('会话重命名拒绝空白标题',async()=>{
  render(<App/>);await screen.findByText('内容 a');
  fireEvent.contextMenu(screen.getByRole('button',{name:/会话 A/}));
  fireEvent.click(await screen.findByRole('menuitem',{name:'重命名'}));
  const dialog=await screen.findByRole('dialog',{name:'重命名会话'});
  fireEvent.change(screen.getByLabelText('会话名称'),{target:{value:'  '}});
  fireEvent.click(within(dialog).getByRole('button',{name:'保存'}));
  expect(await within(dialog).findByRole('alert')).toHaveTextContent('不能为空');
  expect(mocks.command).not.toHaveBeenCalledWith('session_rename',expect.anything());
 });
 it('取消重命名不提交且保留原标题',async()=>{
  render(<App/>);await screen.findByText('内容 a');
  fireEvent.contextMenu(screen.getByRole('button',{name:/会话 A/}));
  fireEvent.click(await screen.findByRole('menuitem',{name:'重命名'}));
  const dialog=await screen.findByRole('dialog',{name:'重命名会话'});
  fireEvent.change(screen.getByLabelText('会话名称'),{target:{value:'临时标题'}});
  fireEvent.click(within(dialog).getByRole('button',{name:'取消'}));
  expect(mocks.command).not.toHaveBeenCalledWith('session_rename',expect.anything());
  expect(screen.getByRole('button',{name:/会话 A/})).toBeInTheDocument();
 });
 it('重命名保存失败时保留对话框和列表标题',async()=>{
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>name==='session_rename'?Promise.reject(new Error('存储失败')):original(name,args));
  render(<App/>);await screen.findByText('内容 a');
  fireEvent.contextMenu(screen.getByRole('button',{name:/会话 A/}));
  fireEvent.click(await screen.findByRole('menuitem',{name:'重命名'}));
  const dialog=await screen.findByRole('dialog',{name:'重命名会话'});
  fireEvent.change(screen.getByLabelText('会话名称'),{target:{value:'新标题'}});
  fireEvent.click(within(dialog).getByRole('button',{name:'保存'}));
  expect(await within(dialog).findByRole('alert')).toHaveTextContent('存储失败');
  expect(screen.getByRole('button',{name:/会话 A/})).toBeInTheDocument();
 });
 it.each(['成功','失败'])('旧的重命名请求%s返回时不影响新打开的对话框',async outcome=>{
  let resolveRename!:(value:unknown)=>void,rejectRename!:(reason:unknown)=>void;
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>{
   if(name==='session_rename'&&args.sessionId==='a')return new Promise((resolve,reject)=>{resolveRename=resolve;rejectRename=reject});
   return original(name,args);
  });
  render(<App/>);await screen.findByText('内容 a');
  fireEvent.contextMenu(screen.getByRole('button',{name:/会话 A/}));
  fireEvent.click(await screen.findByRole('menuitem',{name:'重命名'}));
  let dialog=await screen.findByRole('dialog',{name:'重命名会话'});
  fireEvent.change(screen.getByLabelText('会话名称'),{target:{value:'A 的新标题'}});
  fireEvent.click(within(dialog).getByRole('button',{name:'保存'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('session_rename',{sessionId:'a',title:'A 的新标题'}));
  fireEvent.click(within(dialog).getByRole('button',{name:'取消'}));
  fireEvent.contextMenu(screen.getByRole('button',{name:/会话 B/}));
  fireEvent.click(await screen.findByRole('menuitem',{name:'重命名'}));
  dialog=await screen.findByRole('dialog',{name:'重命名会话'});
  fireEvent.change(screen.getByLabelText('会话名称'),{target:{value:'B 的编辑草稿'}});
  await act(async()=>{
   if(outcome==='成功')resolveRename({id:'a',workspaceId:'w',title:'A 的新标题',createdAt:'',updatedAt:'',parentId:null});
   else rejectRename(new Error('A 保存失败'));
  });
  expect(screen.getByRole('dialog',{name:'重命名会话'})).toBeInTheDocument();
  expect(screen.getByLabelText('会话名称')).toHaveValue('B 的编辑草稿');
  if(outcome==='失败')expect(within(dialog).queryByRole('alert')).not.toBeInTheDocument();
 });
 it('同一标题弹窗正在保存时忽略重复提交',async()=>{
  let resolveRename!:(value:unknown)=>void;
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>name==='session_rename'&&args.sessionId==='a'?new Promise(resolve=>{resolveRename=resolve}):original(name,args));
  render(<App/>);await screen.findByText('内容 a');
  fireEvent.contextMenu(screen.getByRole('button',{name:/会话 A/}));
  fireEvent.click(await screen.findByRole('menuitem',{name:'重命名'}));
  const dialog=await screen.findByRole('dialog',{name:'重命名会话'});
  fireEvent.change(screen.getByLabelText('会话名称'),{target:{value:'新的标题'}});
  const saveButton=within(dialog).getByRole('button',{name:'保存'});
  fireEvent.click(saveButton);fireEvent.click(saveButton);
  await waitFor(()=>expect(mocks.command.mock.calls.filter(([name,args])=>name==='session_rename'&&args.sessionId==='a')).toHaveLength(1));
  await act(async()=>resolveRename({id:'a',workspaceId:'w',title:'新的标题',createdAt:'',updatedAt:'',parentId:null}));
 });
});
