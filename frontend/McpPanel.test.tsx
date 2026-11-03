import {afterEach,beforeEach,describe,expect,it,vi} from 'vitest';
import {cleanup,fireEvent,render,screen,waitFor} from '@testing-library/react';
import {McpPanel} from './McpPanel';
const {command}=vi.hoisted(()=>({command:vi.fn()}));
vi.mock('./api',()=>({desktop:()=>true,command,errorText:(e:unknown)=>e instanceof Error?e.message:String(e)}));
beforeEach(()=>{command.mockReset();command.mockResolvedValue([])});
afterEach(cleanup);
describe('MCP 显式连接',()=>{
 it('不自动连接，HTTP 提交绑定当前工作区并允许显式断开',async()=>{
  let connected=false;
  command.mockImplementation(async(name)=>{
   if(name==='mcp_connect'){connected=true;return}
   if(name==='mcp_disconnect'){connected=false;return}
   return connected?[{id:'remote',workspaceId:'w'}]:[];
  });
  render(<McpPanel workspaceId="w"/>);await screen.findByText('没有活动 MCP 连接。');
  expect(command).not.toHaveBeenCalledWith('mcp_connect',expect.anything());
  fireEvent.change(screen.getByLabelText('连接标识'),{target:{value:'remote'}});
  fireEvent.change(screen.getByLabelText('服务器地址'),{target:{value:'https://example.test/mcp'}});
  fireEvent.click(screen.getByRole('button',{name:'连接 MCP'}));
  await waitFor(()=>expect(command).toHaveBeenCalledWith('mcp_connect',{workspaceId:'w',config:{transport:'streamableHttp',id:'remote',url:'https://example.test/mcp'}}));
  fireEvent.click(await screen.findByRole('button',{name:'断开 remote'}));
  await waitFor(()=>expect(command).toHaveBeenCalledWith('mcp_disconnect',{id:'remote'}));
  await screen.findByText('MCP 连接已断开。');
 });
 it('stdio 显示副本占用说明，校验参数数组并传递真实配置',async()=>{
  render(<McpPanel workspaceId="w"/>);
  expect(screen.getByText(/同步更改前请先断开连接/)).toBeInTheDocument();
  fireEvent.change(screen.getByLabelText('MCP 传输方式'),{target:{value:'stdio'}});
  fireEvent.change(screen.getByLabelText('连接标识'),{target:{value:'local'}});
  fireEvent.change(screen.getByLabelText('可执行程序'),{target:{value:'D:/tools/server.exe'}});
  fireEvent.change(screen.getByLabelText('启动参数（JSON 字符串数组）'),{target:{value:'[1]'}});
  fireEvent.click(screen.getByRole('button',{name:'连接 MCP'}));
  await screen.findByText('启动参数必须是 JSON 字符串数组。');
  expect(command).not.toHaveBeenCalledWith('mcp_connect',expect.anything());
  fireEvent.change(screen.getByLabelText('启动参数（JSON 字符串数组）'),{target:{value:'["--mode", "dev"]'}});
  fireEvent.click(screen.getByRole('button',{name:'连接 MCP'}));
  await waitFor(()=>expect(command).toHaveBeenCalledWith('mcp_connect',{workspaceId:'w',config:{transport:'stdio',id:'local',command:'D:/tools/server.exe',args:['--mode','dev']}}));
 });
 it('连接失败保持错误信息，不显示虚假成功',async()=>{
  command.mockImplementation(async name=>{if(name==='mcp_connect')throw new Error('握手失败');return []});
  render(<McpPanel workspaceId="w"/>);
  fireEvent.change(screen.getByLabelText('连接标识'),{target:{value:'remote'}});
  fireEvent.change(screen.getByLabelText('服务器地址'),{target:{value:'https://example.test/mcp'}});
  fireEvent.click(screen.getByRole('button',{name:'连接 MCP'}));
  await screen.findByText('握手失败');expect(screen.queryByText('MCP 连接已建立。')).not.toBeInTheDocument();
 });
});
