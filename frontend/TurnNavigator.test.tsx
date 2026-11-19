import {cleanup,fireEvent,render,screen} from '@testing-library/react';
import {afterEach,describe,expect,it,vi} from 'vitest';
import {TurnNavigator} from './TurnNavigator';

afterEach(cleanup);
describe('回合导航',()=>{
 it('悬停预览用户消息并支持点击跳转',()=>{
  const jump=vi.fn();
  render(<TurnNavigator entries={[{eventId:'a',seq:'1',preview:'第一条用户消息',previewTruncated:true}]} activeId="a" onJump={jump} onLoadOlder={()=>{}} hasOlder={false} loading={false}/>);
  const button=screen.getByRole('button',{name:'跳转到：第一条用户消息'});
  expect(button.getAttribute('aria-current')).toBe('location');
  fireEvent.mouseEnter(button);
  expect(screen.getByRole('tooltip').textContent).toBe('第一条用户消息…');
  fireEvent.click(button);
  expect(jump).toHaveBeenCalledWith('a');
  expect(screen.queryByRole('tooltip')).toBeNull();
 });
});

it('窄导航按完整字素裁剪文本',async()=>{
 const {fitPreview}=await import('./turnNavigatorText');
 expect(fitPreview('你🧑‍💻好',2,text=>Array.from(new Intl.Segmenter(undefined,{granularity:'grapheme'}).segment(text)).length)).toBe('你🧑‍💻');
});

it('不同会话的历史请求使用独立加载锁',async()=>{
 const {createSessionLoadLock}=await import('./turnNavigatorText');
 const lock=createSessionLoadLock();let resolveA!:(value:boolean)=>void;
 const a=lock.run('a',()=>new Promise<boolean>(resolve=>{resolveA=resolve}));
 const b=lock.run('b',async()=>true);
 expect(await b).toBe(true);resolveA(false);expect(await a).toBe(false);
});

it('跳转等待消息挂载，超时返回空结果',async()=>{
 const {waitForMessage}=await import('./turnNavigatorText');
 const target=document.createElement('div');let calls=0;
 expect(await waitForMessage(()=>++calls===2?target:null,()=>true,3,1)).toBe(target);
 expect(await waitForMessage(()=>null,()=>true,2,1)).toBeNull();
});
