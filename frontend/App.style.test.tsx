import {afterEach,beforeEach,expect,it,vi} from 'vitest';
import {cleanup,fireEvent,render,screen,within} from '@testing-library/react';
import App from './App';
import './style.css';
import {useChat} from './store';
import {useSidePanel} from './sidePanelStore';

const mocks=vi.hoisted(()=>({command:vi.fn()}));
vi.mock('@tauri-apps/api/event',()=>({listen:async()=>()=>{}}));
vi.mock('./api',()=>({desktop:()=>true,command:mocks.command,send:vi.fn(),resume:vi.fn(),errorText:(error:unknown)=>String(error)}));
vi.mock('@tauri-apps/plugin-dialog',()=>({open:vi.fn(),save:vi.fn()}));
vi.mock('@tauri-apps/api/app',()=>({getVersion:vi.fn(async()=>'0.1.6')}));

beforeEach(()=>{
 useChat.setState({events:{},streams:{},statuses:{},timelines:{},timelineLru:[]});useSidePanel.getState().reset();
 mocks.command.mockReset();mocks.command.mockImplementation(async(name:string,args:any)=>{
  if(name==='workspace_list')return [{id:'w',sourcePath:'D:/project',replicaPath:'D:/copy',createdAt:'',name:'project'}];
  if(name==='session_list')return [{id:'a',workspaceId:'w',title:'会话 A',createdAt:'',updatedAt:'2026-11-24T00:00:00Z'}];
  if(name==='session_timeline_page')return {sessionId:args.sessionId,events:[{id:'e',sessionId:args.sessionId,seq:'1',kind:'assistant.message',data:{content:'内容 a'},createdAt:''}],olderCursor:null,throughSeq:'1',hasOlder:false};
  if(name==='approval_mode_get')return 'ask';
  if(name==='session_plan_get')return false;
  return name==='default_selection_get'?null:[];
 });
});
afterEach(cleanup);

it('实际样式下会话行在两种视图中左对齐，归档图标默认隐藏',async()=>{
 render(<App/>);
 const initial=(await screen.findByText('会话 A')).closest('button')!;
 fireEvent.click(initial);await screen.findByText('内容 a');
 expect(initial).toBeVisible();
 expect(screen.getByText('内容 a')).toBeVisible();
 expect(screen.getByLabelText('消息')).toBeVisible();
 expect(screen.getByRole('button',{name:'发送消息'})).toBeVisible();
 expect(getComputedStyle(initial).textAlign).toBe('left');
 expect(getComputedStyle(within(initial).getByLabelText('存档会话')).opacity).toBe('0');
 fireEvent.click(screen.getByRole('switch',{name:'切换视图：项目 / 最近'}));
 const projectRow=within(screen.getByRole('navigation',{name:'会话列表'})).getByText('会话 A').closest('button')!;
 expect(projectRow).toBeVisible();
 expect(getComputedStyle(projectRow).textAlign).toBe('left');
 expect(getComputedStyle(within(projectRow).getByLabelText('存档会话')).opacity).toBe('0');
});

it('实际样式下提问与答复入口可见',async()=>{
 const original=mocks.command.getMockImplementation()!;
 mocks.command.mockImplementation(async(name,args)=>name==='question_list'?[{id:'q',sessionId:'a',question:'请选择',options:[]}]:original(name,args));
 render(<App/>);fireEvent.click((await screen.findByText('会话 A')).closest('button')!);
 const question=await screen.findByRole('dialog',{name:'Agent 需要你的输入'});
 expect(question).toBeVisible();
 expect(within(question).getByLabelText('自定义答复')).toBeVisible();
 expect(within(question).getByRole('button',{name:'提交答复'})).toBeVisible();
});

it('实际样式下审批和拒绝入口可见',async()=>{
 const original=mocks.command.getMockImplementation()!;
 mocks.command.mockImplementation(async(name,args)=>name==='approval_list'?[{id:'approval',sessionId:'a',tool:'shell',arguments:{},reason:'执行命令'}]:original(name,args));
 render(<App/>);
 const approval=await screen.findByRole('dialog',{name:'操作需要你的批准'});
 expect(approval).toBeVisible();
 expect(within(approval).getByRole('button',{name:'批准此次操作'})).toBeVisible();
 expect(within(approval).getByRole('button',{name:'拒绝'})).toBeVisible();
});
