import {afterEach,beforeEach,describe,expect,it,vi} from 'vitest';
import {act,cleanup,fireEvent,render,screen,waitFor} from '@testing-library/react';
import App from './App';
import {useChat} from './store';
import {useSidePanel} from './sidePanelStore';
import type {SessionEvent} from './types';
const mocks=vi.hoisted(()=>({command:vi.fn(),send:vi.fn()}));
vi.mock('./api',()=>({desktop:()=>true,command:mocks.command,send:mocks.send,errorText:(e:unknown)=>e instanceof Error?e.message:String(e)}));
vi.mock('@tauri-apps/plugin-dialog',()=>({open:vi.fn(),save:vi.fn()}));
const event=(id:string,sessionId:string,seq:string,content:string):SessionEvent=>({id,sessionId,seq,kind:'assistant.message',data:{content},createdAt:''});
const latestPage={sessionId:'a',events:[event('latest-4','a','4','最新消息')],olderCursor:'4',throughSeq:'6',hasOlder:true};
const olderPage={sessionId:'a',events:[event('older-1','a','1','更早消息')],olderCursor:null,throughSeq:'6',hasOlder:false};
beforeEach(()=>{useChat.setState({events:{},streams:{},statuses:{},timelines:{},timelineLru:[]});useSidePanel.getState().reset();mocks.command.mockReset();mocks.send.mockReset();
 mocks.command.mockImplementation(async(name:string,args:any)=>{
  if(name==='workspace_list')return [{id:'w',sourcePath:'D:/project',replicaPath:'D:/copy',createdAt:'',name:'project'}];
  if(name==='model_list')return [{id:'m',name:'测试模型',compatibility:'standard',providerId:'custom',apiMode:'chatCompletions',maxOutputTokens:null,baseUrl:'https://example.test',model:'m',credentialRef:'',proxy:null,efforts:['low','high'],effort:null,enabled:true,builtin:false}];
  if(name==='provider_list')return [{id:'custom',name:'自定义供应商',builtin:false,compatibility:'standard',endpoints:[{apiMode:'chatCompletions',baseUrl:'https://example.test'}],defaultModel:'m',credentialRef:'',proxy:null,enabled:true,builtinModels:[],disabledModels:[]}];
  if(name==='default_selection_get')return null;
  if(name==='session_list')return [{id:'a',workspaceId:'w',title:'会话 A'}];
  if(name==='session_timeline_page')return args.beforeSeq==='4'?olderPage:latestPage;
  if(name==='approval_list'||name==='question_list')return [];
  if(name==='approval_mode_get')return 'ask';
  if(name==='session_plan_get')return false;
  return [];
 });
});
afterEach(()=>{cleanup();vi.useRealTimers()});
async function openA(){render(<App/>);fireEvent.click(await screen.findByRole('button',{name:/会话 A/}));await screen.findByText('最新消息')}
describe('会话历史不限页数',()=>{
 it('滚动到顶部自动加载更早历史，到会话开头后显示提示',async()=>{
  await openA();
  const conversation=screen.getByLabelText('聊天消息');
  conversation.scrollTop=0;
  fireEvent.scroll(conversation);

  expect(await screen.findByText('更早消息')).toBeTruthy();
  expect(screen.queryByRole('button',{name:'加载更早消息'})).toBeNull();
  expect(screen.getByText('已到会话开头')).toBeTruthy();
  expect(screen.queryByText(/为控制内存/)).toBeNull();
  expect(screen.queryByText('返回最新消息')).toBeNull();
 });
 it('加载到会话开头后再次滚动不会重复请求更早页',async()=>{
  await openA();
  const conversation=screen.getByLabelText('聊天消息');
  conversation.scrollTop=0;
  fireEvent.scroll(conversation);
  await screen.findByText('更早消息');
  fireEvent.scroll(conversation);

  await waitFor(()=>expect(mocks.command.mock.calls.filter(([name,args])=>name==='session_timeline_page'&&args.beforeSeq==='4')).toHaveLength(1));
 });
 it('实时事件撑出多页时不误报已到会话开头',async()=>{
  const many=Array.from({length:200},(_,index)=>event(`e-${index+1}`,'a',String(index+1),`消息 ${index+1}`)),original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>name==='session_timeline_page'?{sessionId:'a',events:many,olderCursor:null,throughSeq:'200',hasOlder:false}:original(name,args));
  render(<App/>);
  fireEvent.click(await screen.findByRole('button',{name:/会话 A/}));
  await screen.findByText('消息 1');
  await act(async()=>{useChat.getState().receive({type:'durable',event:event('e-201','a','201','新消息')})});

  expect(useChat.getState().timeline('a')?.pages).toHaveLength(2);
  expect(screen.queryByText('已到会话开头')).toBeNull();
  expect(screen.queryByRole('button',{name:'加载更早消息'})).toBeNull();
 });
 it('手动按钮仍可加载更早历史',async()=>{
  await openA();
  fireEvent.click(screen.getByRole('button',{name:'加载更早消息'}));

  expect(await screen.findByText('更早消息')).toBeTruthy();
  expect(screen.getByText('已到会话开头')).toBeTruthy();
 });
});
