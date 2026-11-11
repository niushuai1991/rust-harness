import {afterEach,expect,it,vi} from 'vitest';
import {act,cleanup,render,screen,waitFor} from '@testing-library/react';
import {WebPanel} from './WebPanel';
const mocks=vi.hoisted(()=>({command:vi.fn().mockResolvedValue(undefined),handlers:new Map<string,(event:{payload:any})=>void>()}));
vi.mock('@tauri-apps/api/core',()=>({isTauri:()=>true}));
vi.mock('@tauri-apps/api/event',()=>({listen:async(name:string,handler:(event:{payload:any})=>void)=>{mocks.handlers.set(name,handler);return()=>mocks.handlers.delete(name)}}));
vi.mock('./api',()=>({command:mocks.command,errorText:(e:unknown)=>String(e)}));
afterEach(()=>{cleanup();vi.restoreAllMocks()});
it('网页视图随弹窗隐藏恢复，并在卸载后关闭',async()=>{
 vi.spyOn(Element.prototype,'getBoundingClientRect').mockReturnValue({x:600,y:80,left:600,top:80,right:1000,bottom:800,width:400,height:720,toJSON(){}});
 const {unmount}=render(<WebPanel url="https://example.com"/>);
 await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('web_panel_bounds',{x:600,y:80,width:400,height:720,visible:true}));
 const overlay=document.createElement('div');overlay.className='overlay';document.body.appendChild(overlay);
 await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('web_panel_bounds',{x:600,y:80,width:400,height:720,visible:false}));
 overlay.remove();
 await waitFor(()=>expect(mocks.command.mock.calls.filter(call=>call[0]==='web_panel_bounds'&&call[1].visible)).toHaveLength(2));
 unmount();await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('web_panel_close'));
});
it('系统浏览器打开失败时显示错误',async()=>{
 mocks.command.mockImplementation(async(name:string)=>{if(name==='open_external_link')throw new Error('打开失败')});
 render(<WebPanel url="https://example.com"/>);
 screen.getByRole('button',{name:'浏览器打开'}).click();
 expect(await screen.findByRole('alert')).toHaveTextContent('打开失败');
});

it('忽略旧网页的失败事件，并在当前网页成功后清除错误',async()=>{
 mocks.command.mockResolvedValue(undefined);
 render(<WebPanel url="https://example.com"/>);
 await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('web_panel_open',expect.objectContaining({url:'https://example.com',generation:expect.any(String)})));
 const call=mocks.command.mock.calls.filter(c=>c[0]==='web_panel_open').at(-1)!;
 const generation=call[1].generation;
 await waitFor(()=>expect(mocks.handlers.has('web-panel-load-status')).toBe(true));
 act(()=>mocks.handlers.get('web-panel-load-status')!({payload:['older',false]}));
 expect(screen.queryByRole('alert')).toBeNull();
 act(()=>mocks.handlers.get('web-panel-load-status')!({payload:[generation,false]}));
 expect(screen.getByRole('alert')).toHaveTextContent('网页加载失败');
 act(()=>mocks.handlers.get('web-panel-load-status')!({payload:[generation,true]}));
 expect(screen.queryByRole('alert')).toBeNull();
});
it('网页中的 Escape 请求只关闭当前网页面板',async()=>{
 const {useSidePanel}=await import('./sidePanelStore');
 act(()=>useSidePanel.getState().openWeb('https://example.com'));
 mocks.command.mockResolvedValue(undefined);
 render(<WebPanel url="https://example.com"/>);
 await waitFor(()=>expect(mocks.handlers.has('web-panel-close-request')).toBe(true));
 await waitFor(()=>expect(mocks.command.mock.calls.some(c=>c[0]==='web_panel_open'&&c[1].url==='https://example.com')).toBe(true));
 const generation=mocks.command.mock.calls.filter(c=>c[0]==='web_panel_open').at(-1)![1].generation;
 act(()=>mocks.handlers.get('web-panel-close-request')!({payload:'older'}));
 expect(useSidePanel.getState().open).toBe(true);
 act(()=>mocks.handlers.get('web-panel-close-request')!({payload:generation}));
 expect(useSidePanel.getState().open).toBe(false);
});
