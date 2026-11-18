import {afterEach,beforeEach,describe,expect,it,vi} from 'vitest';
import {act,cleanup,fireEvent,render,screen,waitFor,within} from '@testing-library/react';
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

const turnEvent=(id:string,seq:string,kind:string,data:Record<string,unknown>={}):SessionEvent=>({id,sessionId:'a',seq,kind,data,createdAt:''});
const start=()=>turnEvent('start','1','turn.start');
const user=()=>turnEvent('user','2','user.message',{content:'用户提问'});
const middle=()=>turnEvent('middle','3','assistant.message',{content:'中间说明',reasoning:'秘密思考'});
const attempt=()=>turnEvent('attempt','4','assistant.attempt',{content:'失败半截'});
const tool=()=>turnEvent('tool','5','tool.result',{name:'shell',content:'工具输出'});
const final=()=>turnEvent('final','6','assistant.message',{content:'最终总结'});
const end=(status='completed')=>turnEvent('end','7','turn.end',{status});
async function openEvents(events:SessionEvent[],hasOlder=false){
 const original=mocks.command.getMockImplementation()!;
 mocks.command.mockImplementation(async(name,args)=>name==='session_timeline_page'?{...timeline(args.sessionId,events),hasOlder,olderCursor:hasOlder?events[0].seq:null}:original(name,args));
 render(<App/>);fireEvent.click(await screen.findByRole('button',{name:/会话 A/}));
}
describe('完整回合复制',()=>{
 it('默认收起过程，仅最终助手按钮复制整轮正文',async()=>{
  const copy=installClipboard();await openEvents([start(),user(),middle(),attempt(),tool(),final(),end()]);
  const summary=await screen.findByText('本轮已完成');const details=summary.closest('details')!;
  expect(details.open).toBe(false);expect(screen.getByText('最终总结')).toBeVisible();
  fireEvent.click(summary);expect(details.open).toBe(true);
  expect(within(details).queryByRole('button',{name:'复制消息'})).not.toBeInTheDocument();
  const userCard=screen.getByText('用户提问').closest('article')!;
  fireEvent.click(within(userCard).getByRole('button',{name:'复制消息'}));
  expect(copy).toHaveBeenLastCalledWith('用户提问');
  const finalCard=screen.getByText('最终总结').closest('article')!;
  fireEvent.click(within(finalCard).getByRole('button',{name:'复制消息'}));
  expect(copy).toHaveBeenLastCalledWith('中间说明\n\n最终总结');
 });
 it('最终助手按钮合并所有助手正文，不包含失败内容和工具输出',async()=>{
  const copy=installClipboard();await openEvents([start(),user(),middle(),attempt(),tool(),final(),end()]);
  fireEvent.click(within((await screen.findByText('最终总结')).closest('article')!).getByRole('button',{name:'复制消息'}));
  expect(copy).toHaveBeenCalledWith('中间说明\n\n最终总结');
 });
 it('单条助手消息仅复制自身，无助手消息不显示总结复制按钮',async()=>{
  const copy=installClipboard();await openEvents([start(),user(),final(),end()]);
  fireEvent.click(within((await screen.findByText('最终总结')).closest('article')!).getByRole('button',{name:'复制消息'}));
  expect(copy).toHaveBeenCalledWith('最终总结');
  act(()=>useChat.getState().replaceTimelinePage('a',timeline('a',[start(),user(),tool(),end()])));
  expect(screen.queryByText('最终总结')).not.toBeInTheDocument();
  fireEvent.click(screen.getByText('本轮已完成'));
  expect(screen.getAllByRole('button',{name:'复制消息'})).toHaveLength(1);
 });
 it.each(['error','cancelled'])('异常终态 %s 保持平铺与逐条复制',async(status)=>{
  await openEvents([start(),user(),middle(),attempt(),final(),end(status)]);
  expect(await screen.findByText('中间说明')).toBeVisible();
  expect(screen.queryByText('本轮已完成')).not.toBeInTheDocument();
  expect(screen.getAllByRole('button',{name:'复制消息'})).toHaveLength(4);
 });
 it('运行中平铺，收到完成事件后折叠并复制整轮',async()=>{
  const copy=installClipboard();await openEvents([start(),user(),middle(),final()]);
  expect(await screen.findByText('中间说明')).toBeVisible();
  act(()=>useChat.getState().receive({type:'durable',event:end()}));
  expect(screen.getByText('本轮已完成').closest('details')!.open).toBe(false);
  fireEvent.click(within(screen.getByText('最终总结').closest('article')!).getByRole('button',{name:'复制消息'}));
  expect(copy).toHaveBeenCalledWith('中间说明\n\n最终总结');
 });
 it('跨页回合加载起点前平铺，加载后合并复制',async()=>{
  const copy=installClipboard();await openEvents([middle(),final(),end()],true);
  expect(await screen.findByText('中间说明')).toBeVisible();
  expect(screen.queryByText('本轮已完成')).not.toBeInTheDocument();
  act(()=>useChat.getState().prependTimelinePage('a',timeline('a',[start(),user()])));
  expect(screen.getByText('本轮已完成').closest('details')!.open).toBe(false);
  fireEvent.click(within(screen.getByText('最终总结').closest('article')!).getByRole('button',{name:'复制消息'}));
  expect(copy).toHaveBeenCalledWith('中间说明\n\n最终总结');
 });
});
