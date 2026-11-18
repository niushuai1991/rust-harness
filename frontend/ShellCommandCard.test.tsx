import {afterEach,describe,expect,it} from 'vitest';
import {cleanup,fireEvent,render,screen} from '@testing-library/react';
import {ShellCommandCard} from './ShellCommandCard';
import type {SessionEvent} from './types';

const event=(kind:string,data:Record<string,unknown>):SessionEvent=>({id:kind,sessionId:'s',seq:'1',kind,data,createdAt:''});
const call=event('tool.call',{name:'shell',id:'c',arguments:{command:'echo hello'}});
const result=event('tool.result',{name:'shell',callId:'c',content:JSON.stringify({stdout:'hello\nworld',stderr:'warning',exit_code:0}),status:'completed'});
afterEach(cleanup);

describe('Shell 命令内联行',()=>{
 it('收起为一行摘要，完成态不显示状态词，展开显示命令与结果',()=>{
  render(<ShellCommandCard node={{kind:'shell',key:'c',call,result}}/>);
  const row=screen.getByRole('button',{name:/已执行/});
  expect(row).toHaveAttribute('aria-expanded','false');
  expect(screen.queryByText('已完成')).toBeNull();
  expect(screen.queryByText('退出码 0')).toBeNull();
  fireEvent.click(row);
  expect(row).toHaveAttribute('aria-expanded','true');
  expect(screen.getByText('hello world')).toBeVisible();
  expect(screen.getByText('warning')).toBeVisible();
  expect(screen.getByText('退出码 0')).toBeVisible();
  fireEvent.click(row);
  expect(screen.queryByText('hello world')).toBeNull();
 });
 it('结果未到达时显示正在执行，详情提示等待结果',()=>{
  render(<ShellCommandCard node={{kind:'shell',key:'c',call}}/>);
  expect(screen.getByText('正在执行',{selector:'.shell-kind'})).toBeVisible();
  expect(screen.queryByText('已执行')).toBeNull();
  fireEvent.click(screen.getByRole('button',{name:/正在执行/}));
  expect(screen.getByText('等待结果')).toBeVisible();
 });
 it('长命令可独立展开和收起，保留换行',()=>{
  const long={...call,data:{...call.data,arguments:{command:'one\ntwo\nthree\nfour'}}};
  render(<ShellCommandCard node={{kind:'shell',key:'c',call:long}}/>);
  fireEvent.click(screen.getByRole('button',{name:/正在执行/}));
  const toggle=screen.getByRole('button',{name:'展开命令'});
  expect(toggle).toHaveAttribute('aria-expanded','false');
  fireEvent.click(toggle);
  expect(screen.getByRole('button',{name:'收起'})).toHaveAttribute('aria-expanded','true');
  expect(screen.getByText('one two three four',{selector:'.shell-command-text'}).textContent).toBe('one\ntwo\nthree\nfour');
 });
 it('展示拒绝原因、空输出和附件',()=>{
  const denied={...result,data:{...result.data,status:'denied',isError:true,content:'权限被拒绝'}};
  const view=render(<ShellCommandCard node={{kind:'shell',key:'c',call,result:denied}} artifact={<button>下载完整输出</button>}/>);
  expect(screen.getByText('已拒绝',{selector:'.shell-status'})).toBeVisible();
  fireEvent.click(screen.getByRole('button',{name:/已拒绝/}));
  expect(screen.getByText('权限被拒绝')).toBeVisible();
  expect(screen.getByRole('button',{name:'下载完整输出'})).toBeVisible();
  view.unmount();
  const empty={...result,data:{...result.data,content:'{"stdout":"","stderr":"","exit_code":0}'}};
  render(<ShellCommandCard node={{kind:'shell',key:'c',call,result:empty}}/>);
  fireEvent.click(screen.getByRole('button',{name:/已执行/}));
  expect(screen.getByText('没有输出。')).toBeVisible();
 });
 it('超时或取消后结果未知时不误报确定失败',()=>{
  const unknown={...result,data:{...result.data,status:'unknown',isError:true,content:'结果未知'}};
  render(<ShellCommandCard node={{kind:'shell',key:'c',call,result:unknown}}/>);
  expect(screen.getByText('结果未知',{selector:'.shell-status'})).toBeVisible();
 });
 it('已取消显示取消状态',()=>{
  const cancelled={...result,data:{...result.data,status:'cancelled',content:'{"stdout":"","stderr":"","exit_code":130}'}};
  render(<ShellCommandCard node={{kind:'shell',key:'c',call,result:cancelled}}/>);
  expect(screen.getByText('已取消',{selector:'.shell-status'})).toBeVisible();
 });
 it('命令缺失时显示占位文案',()=>{
  const noCmd={...call,data:{...call.data,arguments:{}}};
  render(<ShellCommandCard node={{kind:'shell',key:'c',call:noCmd,result}}/>);
  expect(screen.getByText('命令不可用')).toBeVisible();
 });
});
