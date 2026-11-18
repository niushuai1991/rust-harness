import {afterEach,beforeEach,describe,expect,it,vi} from 'vitest';
import {cleanup,fireEvent,render,screen,waitFor} from '@testing-library/react';
import App from './App';
import {useChat} from './store';
import {useSidePanel} from './sidePanelStore';
import type {SessionEvent} from './types';
const mocks=vi.hoisted(()=>({command:vi.fn(),send:vi.fn()}));
vi.mock('./api',()=>({desktop:()=>true,command:mocks.command,send:mocks.send,errorText:(e:unknown)=>e instanceof Error?e.message:String(e)}));
vi.mock('@tauri-apps/plugin-dialog',()=>({open:vi.fn(),save:vi.fn()}));
const event=(id:string,sessionId:string,seq:string,content:string):SessionEvent=>({id,sessionId,seq,kind:'assistant.message',data:{content},createdAt:''});
const timeline=(sessionId:string,events:SessionEvent[])=>({sessionId,events,olderCursor:null,throughSeq:events[events.length-1]?.seq??'0',hasOlder:false});
beforeEach(()=>{useChat.setState({events:{},streams:{},statuses:{},timelines:{},timelineLru:[]});useSidePanel.getState().reset();mocks.command.mockReset();mocks.send.mockReset();
 mocks.command.mockImplementation(async(name:string,args:any)=>{
  if(name==='workspace_list')return [{id:'w',sourcePath:'D:/project',replicaPath:'D:/copy',createdAt:'',name:'project'}];
  if(name==='model_list')return [{id:'m',name:'测试模型',compatibility:'standard',providerId:'custom',apiMode:'chatCompletions',maxOutputTokens:null,baseUrl:'https://example.test',model:'m',credentialRef:'',proxy:null,efforts:['low','high'],effort:null,enabled:true,builtin:false}];
  if(name==='provider_list')return [{id:'custom',name:'自定义供应商',builtin:false,compatibility:'standard',endpoints:[{apiMode:'chatCompletions',baseUrl:'https://example.test'}],defaultModel:'m',credentialRef:'',proxy:null,enabled:true,builtinModels:[],disabledModels:[]}];
  if(name==='default_selection_get')return null;
  if(name==='session_list')return [{id:'a',workspaceId:'w',title:'会话 A'}];
  if(name==='session_timeline_page')return timeline(args.sessionId,[event(`e-${args.sessionId}`,args.sessionId,'1','回复内容')]);
  if(name==='approval_list'||name==='question_list')return [];
  if(name==='approval_mode_get')return 'ask';
  if(name==='session_plan_get')return false;
  return [];
 });
});
afterEach(()=>{cleanup();vi.useRealTimers()});
// jsdom 无 clipboard：与 App.test.tsx 链接复制用例同模式注入 mock。
function installClipboard(){
 const copy=vi.fn().mockResolvedValue(undefined);
 Object.defineProperty(navigator,'clipboard',{configurable:true,value:{writeText:copy}});
 return copy;
}
async function openA(){render(<App/>);fireEvent.click(await screen.findByRole('button',{name:/会话 A/}));await screen.findByText('回复内容')}
describe('消息复制按钮',()=>{
 it('助手消息复制按钮位于消息体末尾（左下角），点击复制原文且不改变按钮状态',async()=>{
  const copy=installClipboard();
  await openA();
  const button=await screen.findByRole('button',{name:'复制消息'});
  fireEvent.click(button);
  await waitFor(()=>expect(copy).toHaveBeenCalledWith('回复内容'));
  expect(screen.getByRole('button',{name:'复制消息'})).toBe(button);
  const body=button.closest('.message-body')!;
  expect(body.lastElementChild).toBe(button);
 });
 it('用户消息复制按钮同样位于消息体末尾并复制原文',async()=>{
  const copy=installClipboard();
  const original=mocks.command.getMockImplementation()!;
  mocks.command.mockImplementation(async(name,args)=>name==='session_timeline_page'?timeline(args.sessionId,[
   {id:'u',sessionId:'a',seq:'1',kind:'user.message',data:{content:'用户的问题'},createdAt:''},
   event(`e-${args.sessionId}`,args.sessionId,'2','回复内容'),
  ]):original(name,args));
  render(<App/>);
  fireEvent.click(await screen.findByRole('button',{name:/会话 A/}));
  await screen.findByText('用户的问题');
  const buttons=screen.getAllByRole('button',{name:'复制消息'});
  expect(buttons).toHaveLength(2);
  for(const button of buttons)expect(button.closest('.message-body')!.lastElementChild).toBe(button);
  fireEvent.click(buttons[0]);
  await waitFor(()=>expect(copy).toHaveBeenCalledWith('用户的问题'));
  fireEvent.click(buttons[1]);
  await waitFor(()=>expect(copy).toHaveBeenCalledWith('回复内容'));
 });
});
