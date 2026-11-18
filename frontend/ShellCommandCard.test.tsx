import {afterEach,describe,expect,it} from 'vitest';
import {cleanup,fireEvent,render,screen} from '@testing-library/react';
import {ShellCommandCard} from './ShellCommandCard';
import type {SessionEvent} from './types';

const event=(kind:string,data:Record<string,unknown>):SessionEvent=>({id:kind,sessionId:'s',seq:'1',kind,data,createdAt:''});
const call=event('tool.call',{name:'shell',id:'c',arguments:{command:'echo hello'}});
const result=event('tool.result',{name:'shell',callId:'c',content:JSON.stringify({stdout:'hello\nworld',stderr:'warning',exit_code:0}),status:'completed'});
afterEach(cleanup);

describe('命令卡片',()=>{
 it('默认折叠，展开显示命令及结果，收起后正文不可见',()=>{
  render(<ShellCommandCard node={{kind:'shell',key:'c',call,result}}/>);
  const details=screen.getByText('执行命令').closest('details')!;
  expect(details.open).toBe(false);
  fireEvent.click(screen.getByText('执行命令'));
  expect(details.open).toBe(true);
  expect(screen.getByText('hello world')).toBeVisible();
  expect(screen.getByText('warning')).toBeVisible();
  expect(screen.getByText('退出码 0')).toBeVisible();
  fireEvent.click(screen.getByText('执行命令'));
  expect(details.open).toBe(false);
 });
 it('长命令可独立展开和收起，保留换行',()=>{
  const long={...call,data:{...call.data,arguments:{command:'one\ntwo\nthree\nfour'}}};
  render(<ShellCommandCard node={{kind:'shell',key:'c',call:long}}/>);
  fireEvent.click(screen.getByText('执行命令'));
  const toggle=screen.getByRole('button',{name:'展开完整命令'});
  expect(toggle).toHaveAttribute('aria-expanded','false');
  fireEvent.click(toggle);
  expect(screen.getByRole('button',{name:'收起命令'})).toHaveAttribute('aria-expanded','true');
  expect(screen.getByText('one two three four',{selector:'.shell-command-text'}).textContent).toBe('one\ntwo\nthree\nfour');
 });
 it('展示拒绝原因、空输出和附件',()=>{
  const denied={...result,data:{...result.data,status:'denied',isError:true,content:'权限被拒绝'}};
  const view=render(<ShellCommandCard node={{kind:'shell',key:'c',call,result:denied}} artifact={<button>下载完整输出</button>}/>);
  expect(screen.getByText('已拒绝')).toBeVisible();
  fireEvent.click(screen.getByText('执行命令'));
  expect(screen.getByText('权限被拒绝')).toBeVisible();
  expect(screen.getByRole('button',{name:'下载完整输出'})).toBeVisible();
  view.unmount();
  const empty={...result,data:{...result.data,content:'{"stdout":"","stderr":"","exit_code":0}'}};
  render(<ShellCommandCard node={{kind:'shell',key:'c',call,result:empty}}/>);
  fireEvent.click(screen.getByText('执行命令'));
  expect(screen.getByText('无输出')).toBeVisible();
 });
 it('超时或取消后结果未知时不误报确定失败',()=>{
  const unknown={...result,data:{...result.data,status:'unknown',isError:true,content:'结果未知'}};
  render(<ShellCommandCard node={{kind:'shell',key:'c',call,result:unknown}}/>);
  expect(screen.getByText('结果未知',{selector:'.shell-status'})).toBeVisible();
 });
});
