import {afterEach,beforeEach,expect,it,vi} from 'vitest';
import {act,cleanup,fireEvent,render,screen,waitFor} from '@testing-library/react';
import {PerformancePanel} from './PerformancePanel';

const mocks=vi.hoisted(()=>({command:vi.fn()}));
vi.mock('./api',()=>({command:mocks.command,errorText:(e:unknown)=>e instanceof Error?e.message:String(e)}));

const DEFAULTS={toolConcurrency:true,toolConcurrencyLimit:8,autoCompaction:true,httpConnectionReuse:true};

beforeEach(()=>{mocks.command.mockReset();mocks.command.mockResolvedValue(undefined)});
afterEach(()=>{cleanup()});

it('加载后展示三项开关与上限默认值',async()=>{
 mocks.command.mockImplementation((name:string)=>{
  if(name==='perf_settings_get')return Promise.resolve(DEFAULTS);
  return Promise.resolve(undefined);
 });
 render(<PerformancePanel/>);
 expect(await screen.findByRole('switch',{name:'工具并行调度'})).toBeChecked();
 expect(screen.getByRole('switch',{name:'上下文自动压缩'})).toBeChecked();
 expect(screen.getByRole('switch',{name:'HTTP 连接复用'})).toBeChecked();
 expect(screen.getByLabelText('工具并行数量上限')).toHaveValue(8);
});

it('关闭开关立即保存',async()=>{
 mocks.command.mockImplementation((name:string)=>{
  if(name==='perf_settings_get')return Promise.resolve(DEFAULTS);
  return Promise.resolve(undefined);
 });
 render(<PerformancePanel/>);
 await screen.findByRole('switch',{name:'工具并行调度'});
 await act(async()=>{fireEvent.click(screen.getByRole('switch',{name:'工具并行调度'}))});
 await waitFor(()=>{
  const calls=mocks.command.mock.calls.filter(([name])=>name==='perf_settings_set');
  expect(calls.length).toBe(1);
  expect(calls[0][1]).toEqual({...DEFAULTS,toolConcurrency:false});
 });
 expect(screen.getByRole('switch',{name:'工具并行调度'})).not.toBeChecked();
});

it('上限越界时提示并回退不保存',async()=>{
 mocks.command.mockImplementation((name:string)=>{
  if(name==='perf_settings_get')return Promise.resolve(DEFAULTS);
  return Promise.resolve(undefined);
 });
 render(<PerformancePanel/>);
 const input=await screen.findByLabelText('工具并行数量上限');
 await act(async()=>{fireEvent.change(input,{target:{value:'32'}})});
 await act(async()=>{fireEvent.blur(input)});
 expect(await screen.findByText('工具并发上限必须在 1–16 之间')).toBeInTheDocument();
 expect(mocks.command.mock.calls.filter(([name])=>name==='perf_settings_set').length).toBe(0);
 expect(screen.getByLabelText('工具并行数量上限')).toHaveValue(8);
});

it('上限合法值失焦时保存',async()=>{
 mocks.command.mockImplementation((name:string)=>{
  if(name==='perf_settings_get')return Promise.resolve(DEFAULTS);
  return Promise.resolve(undefined);
 });
 render(<PerformancePanel/>);
 const input=await screen.findByLabelText('工具并行数量上限');
 await act(async()=>{fireEvent.change(input,{target:{value:'4'}})});
 await act(async()=>{fireEvent.blur(input)});
 await waitFor(()=>{
  const calls=mocks.command.mock.calls.filter(([name])=>name==='perf_settings_set');
  expect(calls.length).toBe(1);
  expect(calls[0][1]).toEqual({...DEFAULTS,toolConcurrencyLimit:4});
 });
});

it('加载失败显示错误信息',async()=>{
 mocks.command.mockRejectedValue(new Error('IPC 断开'));
 render(<PerformancePanel/>);
 expect(await screen.findByText('IPC 断开')).toBeInTheDocument();
});
