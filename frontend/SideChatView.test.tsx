import {afterEach,beforeEach,describe,expect,it,vi} from 'vitest';
import {act,cleanup,fireEvent,render,screen,waitFor} from '@testing-library/react';
import {SideChatView} from './SideChatView';
import {useChat} from './store';
import {useSidePanel} from './sidePanelStore';
import type {ApprovalRequest,AskRequest,SessionEvent,SessionInfo} from './types';

const mocks=vi.hoisted(()=>({command:vi.fn(),send:vi.fn(),listeners:new Map<string,Set<(event:{payload:unknown})=>void>>()}));
vi.mock('./api',()=>({command:mocks.command,send:mocks.send,errorText:(e:unknown)=>e instanceof Error?e.message:String(e)}));
vi.mock('@tauri-apps/api/event',()=>({listen:async(name:string,handler:(event:{payload:unknown})=>void)=>{const set=mocks.listeners.get(name)??new Set();set.add(handler);mocks.listeners.set(name,set);return()=>{set.delete(handler);if(!set.size)mocks.listeners.delete(name)}}}));
// 隔离 App 模块：只保留消息内容展示，避免把整个主界面拉进本测试。
vi.mock('./App',()=>({ConversationEvents:({events}:{events:SessionEvent[]})=><div data-testid="events">{events.map(event=><div key={event.id}>{String(event.data?.content??'')}</div>)}</div>}));

const sessions:SessionInfo[]=[{id:'side-1',workspaceId:'w',title:'会话 A · 侧聊',createdAt:'',updatedAt:'',parentId:'a'},{id:'a',workspaceId:'w',title:'会话 A',createdAt:'',updatedAt:''}];
function baseProps(overrides:Partial<Parameters<typeof SideChatView>[0]>={}) {
 return {sessionId:'side-1',parentSessionId:'a',workspaceId:'w',modelId:'m',effort:null,sessions,activeIds:new Set<string>(),approvals:[] as ApprovalRequest[],questions:[] as AskRequest[],onResolveApproval:vi.fn(),onResolveQuestion:vi.fn(),onOpenSession:vi.fn(),...overrides};
}
function page(sessionId:string,events:SessionEvent[]=[],options:{olderCursor?:string|null;hasOlder?:boolean}={}){return {sessionId,events,olderCursor:options.olderCursor??null,throughSeq:events.at(-1)?.seq??'0',hasOlder:options.hasOlder??false}}
beforeEach(()=>{
 useChat.setState({events:{},timelines:{},timelineLru:[],pinnedTimelineIds:[],streams:{},statuses:{}});
 useSidePanel.getState().reset();
 mocks.listeners.clear();mocks.command.mockReset();mocks.send.mockReset();
 mocks.command.mockImplementation(async(name:string,args:Record<string,unknown>)=>name==='session_timeline_page'?page(String(args.sessionId)):[]);
});
afterEach(cleanup);

describe('侧聊视图',()=>{
 it('打开时拉取侧聊时间线并显示空白起点提示',async()=>{
  render(<SideChatView {...baseProps()}/>);
  expect(await screen.findByText('侧聊从空白开始')).toBeInTheDocument();
  expect(mocks.command).toHaveBeenCalledWith('session_timeline_page',{sessionId:'side-1'});
  expect(screen.getByText(/来自「/)).toBeInTheDocument();
 });

 it('发送纯文本消息并清空草稿',async()=>{
  mocks.send.mockResolvedValue(undefined);
  render(<SideChatView {...baseProps()}/>);
  const input=await screen.findByLabelText('侧聊消息');
  fireEvent.change(input,{target:{value:'顺便问个问题'}});
  fireEvent.click(screen.getByRole('button',{name:'发送侧聊消息'}));
  await waitFor(()=>expect(mocks.send).toHaveBeenCalledWith('side-1','顺便问个问题','m',expect.any(Function),undefined,null));
  expect(input).toHaveValue('');
 });

 it('流式增量与落库消息都渲染在抽屉时间线',async()=>{
  let finish:()=>void=()=>{};
  mocks.send.mockReturnValue(new Promise<void>(resolve=>{finish=resolve}));
  render(<SideChatView {...baseProps()}/>);
  const input=await screen.findByLabelText('侧聊消息');
  fireEvent.change(input,{target:{value:'流式问题'}});
  fireEvent.click(screen.getByRole('button',{name:'发送侧聊消息'}));
  await waitFor(()=>expect(mocks.send).toHaveBeenCalled());
  const onEvent=mocks.send.mock.calls[0][3] as (event:unknown)=>void;
  act(()=>onEvent({type:'delta',sessionId:'side-1',requestId:'r',index:0,text:'流式回答',reasoning:false}));
  expect(await screen.findByText('流式回答')).toBeInTheDocument();
  act(()=>onEvent({type:'durable',event:{id:'m1',sessionId:'side-1',seq:'1',kind:'assistant.message',data:{content:'最终回答'},createdAt:''}}));
  expect(await screen.findByText('最终回答')).toBeInTheDocument();
  await act(async()=>finish());
 });

 it('运行中发送作为转向，停止按钮取消当前回合',async()=>{
  const turnStart:SessionEvent={id:'t1',sessionId:'side-1',seq:'1',kind:'turn.start',data:{runId:'run-1'},createdAt:''};
  mocks.command.mockImplementation(async(name:string,args:Record<string,unknown>)=>name==='session_timeline_page'?page(String(args.sessionId),[turnStart]):[]);
  render(<SideChatView {...baseProps({activeIds:new Set(['side-1'])})}/>);
  const input=await screen.findByLabelText('侧聊消息');
  fireEvent.change(input,{target:{value:'补充一句'}});
  fireEvent.click(screen.getByRole('button',{name:'发送侧聊消息'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('session_steer',{sessionId:'side-1',expectedRunId:'run-1',messageId:expect.any(String),text:'补充一句'}));
  fireEvent.click(screen.getByRole('button',{name:'取消生成'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('session_cancel',{sessionId:'side-1'}));
 });

 it('侧聊审批与提问在抽屉内处理',async()=>{
  const approvals:ApprovalRequest[]=[{id:'ap',sessionId:'side-1',tool:'shell',reason:'要执行命令',arguments:{command:'dir'}}];
  const questions:AskRequest[]=[{id:'q1',sessionId:'side-1',question:'选哪个？',options:[{label:'A'}]}];
  const props=baseProps({approvals,questions});
  render(<SideChatView {...props}/>);
  expect(await screen.findByRole('dialog',{name:'操作需要你的批准'})).toBeInTheDocument();
  fireEvent.click(screen.getByRole('button',{name:'批准此次操作'}));
  expect(props.onResolveApproval).toHaveBeenCalledWith('ap',true);
  fireEvent.click(screen.getByLabelText('选项 A'));
  fireEvent.click(screen.getByRole('button',{name:'提交答复'}));
  expect(props.onResolveQuestion).toHaveBeenCalledWith('q1',{text:'A',source:'selected'});
 });

 it('提问挂起时输入框发送按文本答复而不是新消息',async()=>{
  const questions:AskRequest[]=[{id:'q2',sessionId:'side-1',question:'继续吗？',options:[]}];
  const props=baseProps({questions});
  render(<SideChatView {...props}/>);
  const input=await screen.findByLabelText('侧聊消息');
  fireEvent.change(input,{target:{value:'继续'}});
  fireEvent.click(screen.getByRole('button',{name:'发送侧聊消息'}));
  expect(props.onResolveQuestion).toHaveBeenCalledWith('q2',{text:'继续',source:'text'});
  expect(mocks.send).not.toHaveBeenCalled();
  expect(input).toHaveValue('');
 });

 it('打开时携带的首条文字只发送一次',async()=>{
  mocks.send.mockResolvedValue(undefined);
  act(()=>useSidePanel.getState().openChat('side-1','a','首条问题'));
  render(<SideChatView {...baseProps()}/>);
  await waitFor(()=>expect(mocks.send).toHaveBeenCalledTimes(1));
  expect(mocks.send).toHaveBeenCalledWith('side-1','首条问题','m',expect.any(Function),undefined,null);
  expect(useSidePanel.getState().chatInitial).toBeNull();
 });

 it('加载更早消息使用游标并在到达开头后提示',async()=>{
  mocks.command.mockImplementation(async(name:string,args:Record<string,unknown>)=>{
   if(name!=='session_timeline_page')return [];
   if(args.beforeSeq)return page('side-1',[],{olderCursor:null,hasOlder:false});
   return page('side-1',[],{olderCursor:'50',hasOlder:true});
  });
  render(<SideChatView {...baseProps()}/>);
  fireEvent.click(await screen.findByRole('button',{name:'加载更早消息'}));
  await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('session_timeline_page',{sessionId:'side-1',beforeSeq:'50'}));
  expect(await screen.findByText('已到侧聊开头')).toBeInTheDocument();
 });

 it('侧聊回合结束事件触发补拉最新时间线',async()=>{
  render(<SideChatView {...baseProps()}/>);
  await waitFor(()=>expect(mocks.listeners.has('session-steering-event')).toBe(true));
  const calls=mocks.command.mock.calls.filter(([name])=>name==='session_timeline_page').length;
  for(const handler of [...(mocks.listeners.get('session-steering-event')??[])])await act(async()=>handler({payload:{id:'end',sessionId:'side-1',seq:'9',kind:'turn.end',data:{status:'completed'},createdAt:''}}));
  await waitFor(()=>expect(mocks.command.mock.calls.filter(([name])=>name==='session_timeline_page').length).toBeGreaterThan(calls));
 });
});
