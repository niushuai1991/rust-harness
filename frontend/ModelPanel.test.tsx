import {afterEach,beforeEach,expect,it,vi} from 'vitest';
import {act,cleanup,fireEvent,render,screen,waitFor} from '@testing-library/react';
import App from './App';
const mocks=vi.hoisted(()=>({command:vi.fn()}));
vi.mock('./api',()=>({desktop:()=>true,command:mocks.command,send:vi.fn(),errorText:(e:unknown)=>e instanceof Error?e.message:String(e)}));
vi.mock('@tauri-apps/plugin-dialog',()=>({open:vi.fn(),save:vi.fn()}));
const providers=[{id:'deepseek',name:'DeepSeek',builtin:true,compatibility:'deepSeek',endpoints:[{apiMode:'chatCompletions',baseUrl:'https://api.deepseek.com'},{apiMode:'responses',baseUrl:'https://api.deepseek.com'},{apiMode:'anthropic',baseUrl:'https://api.deepseek.com/anthropic'}],defaultModel:'deepseek-flash',credentialRef:'deep-key',proxy:null},{id:'zhipu',name:'智谱 Coding Plan',builtin:true,compatibility:'bigModel',endpoints:[{apiMode:'chatCompletions',baseUrl:'https://open.bigmodel.cn/api/coding/paas/v4'},{apiMode:'responses',baseUrl:'https://open.bigmodel.cn/api/coding/paas/v4'},{apiMode:'anthropic',baseUrl:'https://open.bigmodel.cn/api/anthropic'}],defaultModel:'glm-5.3',credentialRef:'glm-key',proxy:null}];
let saved:any[]=[],catalog=structuredClone(providers);
beforeEach(()=>{saved=[];catalog=structuredClone(providers);mocks.command.mockReset();mocks.command.mockImplementation(async(name:string,args:any)=>{
 if(name==='provider_list')return structuredClone(catalog);
 if(name==='model_list')return saved;
 if(name==='provider_save'){catalog=[...catalog.filter(p=>p.id!==args.config.id),args.config];return args.config;}
 if(name==='model_save'){saved=[args.config];return null}
 if(name==='model_test')throw new Error('服务返回 401');
 if(name==='provider_copy'){const copy={...structuredClone(providers[0]),id:'copy',name:'DeepSeek 副本',builtin:false,credentialRef:''};catalog.push(copy);return copy;}
 return [];
})});
afterEach(cleanup);
async function openPanel(){render(<App/>);fireEvent.click(screen.getByRole('button',{name:/模型设置/}));await screen.findByRole('button',{name:'DeepSeek'})}
it('供应商仅有两个内置项，协议切换显示固定端点并清除跨供应商密钥',async()=>{
 await openPanel();expect(screen.getAllByRole('button',{name:/^(DeepSeek|智谱 Coding Plan)$/})).toHaveLength(2);
 expect(screen.getByLabelText('模型标识')).toHaveValue('deepseek-flash');
 expect(screen.getByLabelText('API 地址')).toHaveAttribute('readonly');
 fireEvent.change(screen.getByLabelText('协议'),{target:{value:'anthropic'}});
 expect(screen.getByLabelText('API 地址')).toHaveValue('https://api.deepseek.com/anthropic');
 fireEvent.change(screen.getByLabelText('API 密钥'),{target:{value:'temporary-key'}});
 fireEvent.click(screen.getByRole('button',{name:'智谱 Coding Plan'}));
 expect(screen.getByLabelText('API 密钥')).toHaveValue('');expect(screen.getByLabelText('模型标识')).toHaveValue('glm-5.3');
});
it('复制供应商使用独立端点且不带入输入中的密钥',async()=>{
 await openPanel();fireEvent.change(screen.getByLabelText('API 密钥'),{target:{value:'unsaved-secret'}});
 fireEvent.click(screen.getByRole('button',{name:'复制供应商'}));await screen.findByRole('button',{name:'DeepSeek 副本'});
 expect(screen.getByLabelText('API 密钥')).toHaveValue('');expect(screen.getByLabelText('API 地址')).not.toHaveAttribute('readonly');
 fireEvent.change(screen.getByLabelText('协议'),{target:{value:'anthropic'}});fireEvent.change(screen.getByLabelText('API 地址'),{target:{value:'https://custom.test/anthropic'}});
 fireEvent.change(screen.getByLabelText('协议'),{target:{value:'chatCompletions'}});expect(screen.getByLabelText('API 地址')).toHaveValue('https://api.deepseek.com');
 fireEvent.change(screen.getByLabelText('协议'),{target:{value:'anthropic'}});expect(screen.getByLabelText('API 地址')).toHaveValue('https://custom.test/anthropic');
 fireEvent.click(screen.getByRole('button',{name:'DeepSeek'}));fireEvent.change(screen.getByLabelText('协议'),{target:{value:'anthropic'}});expect(screen.getByLabelText('API 地址')).toHaveValue('https://api.deepseek.com/anthropic');
});
it('先保存供应商和模型才能测试，失败显示错误且编辑后禁止测试旧路由',async()=>{
 await openPanel();expect(screen.getByRole('button',{name:'测试已保存配置'})).toBeDisabled();
 fireEvent.click(screen.getByRole('button',{name:'保存模型'}));await screen.findByText(/已保存；/);
 fireEvent.click(screen.getByRole('button',{name:'测试已保存配置'}));await screen.findByRole('alert');expect(screen.getByRole('alert')).toHaveTextContent('服务返回 401');expect(screen.queryByText('模型连接成功。')).not.toBeInTheDocument();
 expect(mocks.command.mock.calls.findIndex(c=>c[0]==='provider_save')).toBeLessThan(mocks.command.mock.calls.findIndex(c=>c[0]==='model_save'));
 fireEvent.change(screen.getByLabelText('模型标识'),{target:{value:'another-model'}});expect(screen.getByRole('button',{name:'测试已保存配置'})).toBeDisabled();
});
it('未保存的供应商端点草稿不能通过重新选择已保存模型绕过保存检查',async()=>{
 await openPanel();fireEvent.click(screen.getByRole('button',{name:'复制供应商'}));await screen.findByRole('button',{name:'DeepSeek 副本'});
 fireEvent.click(screen.getByRole('button',{name:'保存模型'}));await screen.findByText(/已保存；/);
 fireEvent.change(screen.getByLabelText('API 地址'),{target:{value:'https://unsaved.test/v1'}});
 fireEvent.click(screen.getByRole('button',{name:'DeepSeek · deepseek-flash'}));
 expect(screen.getByRole('button',{name:'测试已保存配置'})).toBeDisabled();
});
it('新自定义供应商可以只填写所需协议，保存后成为独立模型',async()=>{
 await openPanel();fireEvent.click(screen.getByRole('button',{name:'添加自定义供应商'}));
 fireEvent.change(screen.getByLabelText('供应商名称'),{target:{value:'私有供应商'}});
 fireEvent.change(screen.getByLabelText('API 地址'),{target:{value:'https://private.test/v1'}});
 fireEvent.change(screen.getByLabelText('模型标识'),{target:{value:'private-model'}});
 fireEvent.change(screen.getByLabelText('最大输出 Token（可选）'),{target:{value:'4096'}});
 fireEvent.click(screen.getByRole('button',{name:'保存模型'}));await screen.findByText(/已保存；/);
 expect(mocks.command).toHaveBeenCalledWith('provider_save',{config:expect.objectContaining({defaultModel:'private-model',compatibility:'standard'}),secret:undefined});
 expect(mocks.command).toHaveBeenCalledWith('model_save',{config:expect.objectContaining({model:'private-model',maxOutputTokens:4096,apiMode:'chatCompletions',baseUrl:'https://private.test/v1'})});
});
it('连接测试的迟到成功不能覆盖切换后的供应商状态',async()=>{
 let resolve!:(value:string)=>void;const original=mocks.command.getMockImplementation()!;
 mocks.command.mockImplementation((name,args)=>name==='model_test'?new Promise(r=>{resolve=r}):original(name,args));
 await openPanel();fireEvent.click(screen.getByRole('button',{name:'保存模型'}));await screen.findByText(/已保存；/);
 fireEvent.click(screen.getByRole('button',{name:'测试已保存配置'}));fireEvent.click(screen.getByRole('button',{name:'智谱 Coding Plan'}));
 await act(async()=>resolve('ok'));expect(screen.queryByText('模型连接成功。')).not.toBeInTheDocument();expect(screen.getByLabelText('模型标识')).toHaveValue('glm-5.3');
});
it('外部代理变更阻止覆盖并明确要求重新加载',async()=>{
 await openPanel();const original=mocks.command.getMockImplementation()!;
 mocks.command.mockImplementation((name,args)=>name==='provider_list'?Promise.resolve(providers.map(p=>({...p,proxy:'http://changed.test:7890'}))):original(name,args));
 fireEvent.click(screen.getByRole('button',{name:'保存模型'}));await screen.findByRole('alert');expect(screen.getByRole('alert')).toHaveTextContent('其他位置更改');expect(mocks.command).not.toHaveBeenCalledWith('provider_save',expect.anything());
});

it('关联模型阻止删除供应商，删除模型后才允许移除供应商',async()=>{
 const original=mocks.command.getMockImplementation()!;
 mocks.command.mockImplementation((name,args)=>{if(name==='model_remove'){saved=[];return Promise.resolve(null)}if(name==='provider_remove'){catalog=catalog.filter(p=>p.id!==args.id);return Promise.resolve(null)}return original(name,args)});
 await openPanel();fireEvent.click(screen.getByRole('button',{name:'复制供应商'}));await screen.findByRole('button',{name:'DeepSeek 副本'});
 fireEvent.click(screen.getByRole('button',{name:'保存模型'}));await screen.findByText(/已保存；/);expect(screen.getByRole('button',{name:'删除供应商'})).toBeDisabled();
 fireEvent.click(screen.getByRole('button',{name:'删除模型'}));await waitFor(()=>expect(screen.getByRole('button',{name:'删除供应商'})).not.toBeDisabled());
 fireEvent.click(screen.getByRole('button',{name:'删除供应商'}));await waitFor(()=>expect(screen.queryByRole('button',{name:'DeepSeek 副本'})).not.toBeInTheDocument());
 expect(mocks.command).toHaveBeenCalledWith('provider_remove',{id:'copy'});expect(screen.getByRole('button',{name:'DeepSeek'})).toBeInTheDocument();
});
it('删除未保存自定义供应商只丢弃本地草稿',async()=>{
 await openPanel();fireEvent.click(screen.getByRole('button',{name:'添加自定义供应商'}));fireEvent.click(screen.getByRole('button',{name:'删除供应商'}));
 await waitFor(()=>expect(screen.queryByRole('button',{name:'自定义供应商'})).not.toBeInTheDocument());expect(mocks.command).not.toHaveBeenCalledWith('provider_remove',expect.anything());
});

it('复制 DeepSeek 为自定义供应商后保存模型仍携带其兼容策略',async()=>{
 await openPanel();fireEvent.click(screen.getByRole('button',{name:'复制供应商'}));await screen.findByRole('button',{name:'DeepSeek 副本'});
 fireEvent.change(screen.getByLabelText('协议'),{target:{value:'responses'}});
 fireEvent.click(screen.getByRole('button',{name:'保存模型'}));await screen.findByText(/已保存；/);
 expect(mocks.command).toHaveBeenCalledWith('model_save',{config:expect.objectContaining({providerId:'copy',apiMode:'responses',compatibility:'deepSeek'})});
});
it('复制供应商保留当前模型参数但分配新模型标识且清除密钥',async()=>{
 await openPanel();fireEvent.click(screen.getByRole('button',{name:'保存模型'}));await screen.findByText(/已保存；/);
 const oldModelId=saved[0].id;
 fireEvent.change(screen.getByLabelText('协议'),{target:{value:'responses'}});
 fireEvent.change(screen.getByLabelText('模型标识'),{target:{value:'custom-reasoner'}});
 fireEvent.change(screen.getByLabelText('显示名称'),{target:{value:'推理配置'}});
 fireEvent.change(screen.getByLabelText('最大输出 Token（可选）'),{target:{value:'8192'}});
 fireEvent.change(screen.getByLabelText('API 密钥'),{target:{value:'never-copy-secret'}});
 fireEvent.click(screen.getByRole('button',{name:'复制供应商'}));await screen.findByRole('button',{name:'DeepSeek 副本'});
 expect(screen.getByLabelText('协议')).toHaveValue('responses');expect(screen.getByLabelText('模型标识')).toHaveValue('custom-reasoner');expect(screen.getByLabelText('显示名称')).toHaveValue('推理配置');expect(screen.getByLabelText('最大输出 Token（可选）')).toHaveValue(8192);expect(screen.getByLabelText('API 密钥')).toHaveValue('');
 expect(screen.getByRole('button',{name:'测试已保存配置'})).toBeDisabled();expect(saved).toHaveLength(1);
 fireEvent.click(screen.getByRole('button',{name:'保存模型'}));await screen.findByText(/已保存；/);
 expect(saved[0]).toMatchObject({providerId:'copy',apiMode:'responses',model:'custom-reasoner',name:'推理配置',maxOutputTokens:8192,credentialRef:'',compatibility:'deepSeek',proxy:null});expect(saved[0].id).not.toBe(oldModelId);
 const copiedSave=mocks.command.mock.calls.filter(c=>c[0]==='provider_save').at(-1);expect(copiedSave?.[1].secret).toBeUndefined();
});
