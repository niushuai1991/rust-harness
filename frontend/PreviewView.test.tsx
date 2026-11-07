import {afterEach,beforeEach,describe,expect,it,vi} from 'vitest';
import {act,cleanup,fireEvent,render,screen} from '@testing-library/react';
import {PreviewView} from './PreviewView';
const mocks=vi.hoisted(()=>({command:vi.fn()}));
vi.mock('./api',()=>({desktop:()=>true,command:mocks.command,errorText:(e:unknown)=>e instanceof Error?e.message:String(e)}));
beforeEach(()=>{mocks.command.mockReset()});
afterEach(cleanup);

describe('文件预览视图',()=>{
 it('渲染内容、行号与路径，截断时提示完整大小',async()=>{
  mocks.command.mockResolvedValue({path:'src/app.ts',content:'const a=1;\nconst b=2;\n',truncated:true,binary:false,sizeBytes:40960});
  const {container}=render(<PreviewView workspaceId="w" path="src/app.ts"/>);
  await screen.findByText('src/app.ts');
  expect(screen.getByText(/仅显示前 32 KiB，文件更大（共 40 KiB）/)).toBeInTheDocument();
  expect(container.querySelector('.preview-gutter')?.textContent).toBe('123');
  expect(screen.getByText(/const b=2;/)).toBeInTheDocument();
  expect(mocks.command).toHaveBeenCalledWith('workspace_file_preview',{workspaceId:'w',path:'src/app.ts'});
 });
 it('二进制文件显示占位而不是半截内容',async()=>{
  mocks.command.mockResolvedValue({path:'img.png',content:'',truncated:false,binary:true,sizeBytes:2048});
  render(<PreviewView workspaceId="w" path="img.png"/>);
  await screen.findByText(/二进制文件，暂不支持预览（2048 字节）/);
  expect(screen.queryByText(/const/)).not.toBeInTheDocument();
 });
 it('读取失败显示错误且不假装成功',async()=>{
  mocks.command.mockRejectedValue(new Error('读取失败'));
  render(<PreviewView workspaceId="w" path="missing.ts"/>);
  await screen.findByText('读取失败');
 });
 it('未选择文件时显示引导空态，不发请求',()=>{
  render(<PreviewView workspaceId="w" path={null}/>);
  expect(screen.getByText(/从聊天流的工具卡点击文件名/)).toBeInTheDocument();
  expect(mocks.command).not.toHaveBeenCalled();
 });
 it('切换文件重新读取',async()=>{
  mocks.command.mockResolvedValue({path:'a.ts',content:'A',truncated:false,binary:false,sizeBytes:1});
  const {rerender}=render(<PreviewView workspaceId="w" path="a.ts"/>);
  await screen.findByText('A');
  mocks.command.mockResolvedValue({path:'b.ts',content:'B',truncated:false,binary:false,sizeBytes:1});
  rerender(<PreviewView workspaceId="w" path="b.ts"/>);
  await screen.findByText('B');
  expect(mocks.command).toHaveBeenCalledWith('workspace_file_preview',{workspaceId:'w',path:'b.ts'});
 });
 it('路径变化期间显示读取中状态',async()=>{
  let release!:(value:unknown)=>void;
  mocks.command.mockImplementation(()=>new Promise(resolve=>{release=resolve}));
  render(<PreviewView workspaceId="w" path="slow.ts"/>);
  await screen.findByText('正在读取文件…');
  await act(async()=>release({path:'slow.ts',content:'x',truncated:false,binary:false,sizeBytes:1}));
  expect(screen.queryByText('正在读取文件…')).not.toBeInTheDocument();
 });
});
