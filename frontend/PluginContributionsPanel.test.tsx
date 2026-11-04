import {afterEach,expect,it,vi} from 'vitest';
import {cleanup,fireEvent,render,screen,waitFor} from '@testing-library/react';
import {StrictMode} from 'react';
import {PluginContributionsPanel,PluginThemeHost} from './PluginContributionsPanel';
import {usePluginThemes} from './pluginTypes';
import {PluginResultCards} from './PluginResultCards';
const {command}=vi.hoisted(()=>({command:vi.fn()}));
vi.mock('./api',()=>({desktop:()=>true,command,errorText:(e:unknown)=>String(e)}));
afterEach(()=>{cleanup();command.mockReset();usePluginThemes.setState({selections:{}});document.documentElement.removeAttribute('style')});
const plugin={id:'sample',name:'Sample',settings:{enabled:{type:'boolean',default:true},count:{type:'number',minimum:1,maximum:5},color:{type:'string',enum:['red','blue']}},values:{enabled:true,count:2,color:'red'},commands:[{name:'run',title:'运行',parameters:{type:'object',properties:{message:{type:'string'}},required:['message']}}],themes:{dark:{'--accent':'#aabbcc'}}};
it('renders real settings, saves scoped values and executes declared command arguments',async()=>{
 command.mockImplementation(async(name)=>name==='plugin_contributions'?[plugin]:{content:'{"cards":[{"type":"text","text":"done"}]}'});
 render(<StrictMode><PluginContributionsPanel workspaceId="w" sessionId="s"/></StrictMode>);
 fireEvent.change(await screen.findByLabelText('count'),{target:{value:'3'}});
 fireEvent.click(screen.getByRole('button',{name:'保存 Sample 设置'}));
 await waitFor(()=>expect(command).toHaveBeenCalledWith('plugin_settings_save',{workspaceId:'w',pluginId:'sample',values:{enabled:true,count:3,color:'red'}}));
 fireEvent.change(screen.getByLabelText('message'),{target:{value:'hello'}});
 fireEvent.click(screen.getByRole('button',{name:'运行'}));
 await waitFor(()=>expect(command).toHaveBeenCalledWith('plugin_command',{sessionId:'s',pluginId:'sample',name:'run',params:{message:'hello'}}));
 await screen.findByText('done');
});
it('applies theme variables and resets when contributions disappear',async()=>{
 command.mockResolvedValue([plugin]);let view=render(<><PluginThemeHost workspaceId="w"/><PluginContributionsPanel workspaceId="w"/></>);fireEvent.click(await screen.findByRole('button',{name:'应用 dark'}));expect(document.documentElement.style.getPropertyValue('--accent')).toBe('#aabbcc');
 view.rerender(<PluginThemeHost workspaceId="w"/>);expect(document.documentElement.style.getPropertyValue('--accent')).toBe('#aabbcc');command.mockResolvedValue([]);view.rerender(<PluginThemeHost workspaceId="w" revision={1}/>);await waitFor(()=>expect(document.documentElement.style.getPropertyValue('--accent')).toBe(''));
});
it('renders untrusted text literally and only previews safe replica files after a click',async()=>{
 command.mockResolvedValue('preview');let view=render(<PluginResultCards workspaceId="w" content={JSON.stringify({cards:[{type:'text',text:'<img src=x onerror=alert(1)>'},{type:'table',columns:['a'],rows:[['<script>bad</script>']]},{type:'file',path:'src/main.rs'},{type:'file',path:'../secret'}]})}/>);
 expect(view.container.querySelector('img,script')).toBeNull();expect(command).not.toHaveBeenCalled();fireEvent.click(screen.getByRole('button',{name:'预览 src/main.rs'}));await screen.findByText('preview');expect(command).toHaveBeenCalledWith('plugin_file_preview',{workspaceId:'w',path:'src/main.rs'});expect(screen.queryByRole('button',{name:'预览 ../secret'})).toBeNull();
});
