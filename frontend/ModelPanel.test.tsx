import {afterEach,beforeEach,expect,it,vi} from 'vitest';
import {act,cleanup,fireEvent,render,screen,waitFor,within} from '@testing-library/react';
import App from './App';
const mocks=vi.hoisted(()=>({command:vi.fn()}));
vi.mock('./api',()=>({desktop:()=>true,command:mocks.command,send:vi.fn(),errorText:(e:unknown)=>e instanceof Error?e.message:String(e)}));
vi.mock('@tauri-apps/plugin-dialog',()=>({open:vi.fn(),save:vi.fn()}));
const baseProvider={enabled:true,builtinModels:[],disabledModels:[]};
const deepseek={...baseProvider,id:'deepseek',name:'DeepSeek',builtin:true,compatibility:'deepSeek',endpoints:[{apiMode:'chatCompletions',baseUrl:'https://api.deepseek.com'},{apiMode:'responses',baseUrl:'https://api.deepseek.com'},{apiMode:'anthropic',baseUrl:'https://api.deepseek.com/anthropic'}],defaultModel:'deepseek-flash',credentialRef:'deep-key',proxy:null,
 builtinModels:[{id:'deepseek-flash',name:'DeepSeek Flash',efforts:['disabled','enabled']},{id:'deepseek-v4-pro',name:'DeepSeek V4 Pro',efforts:['disabled','enabled']}]};
const zhipu={...deepseek,id:'zhipu',name:'智谱 Coding Plan',compatibility:'bigModel',endpoints:[{apiMode:'chatCompletions',baseUrl:'https://open.bigmodel.cn/api/coding/paas/v4'},{apiMode:'responses',baseUrl:'https://open.bigmodel.cn/api/coding/paas/v4'},{apiMode:'anthropic',baseUrl:'https://open.bigmodel.cn/api/anthropic'}],defaultModel:'glm-5.3',credentialRef:'glm-key',
 builtinModels:[{id:'glm-5.3',name:'GLM-5.3',efforts:['low','high','max']}]};
let catalog:any[]=[],savedModels:any[]=[];
function reset(){catalog=[structuredClone(deepseek),structuredClone(zhipu)];savedModels=[]}
beforeEach(()=>{reset();mocks.command.mockReset();mocks.command.mockImplementation(async(name:string,args:any)=>{
 if(name==='provider_list')return structuredClone(catalog);
 if(name==='model_list')return structuredClone(savedModels);
 if(name==='default_selection_get')return null;
 if(name==='provider_save'){catalog=[...catalog.filter(p=>p.id!==args.config.id),structuredClone(args.config)];return structuredClone(args.config)}
 if(name==='model_save'){const next=structuredClone(args.config);savedModels=[...savedModels.filter(m=>m.id!==next.id),next];return null}
 if(name==='model_remove'){savedModels=savedModels.filter(m=>m.id!==args.modelId);return null}
 if(name==='model_test')throw new Error('服务返回 401');
 if(name==='provider_copy'){const copy={...structuredClone(deepseek),id:'copy',name:'DeepSeek 副本',builtin:false,credentialRef:''};catalog.push(copy);return structuredClone(copy)}
 if(name==='provider_remove'){catalog=catalog.filter(p=>p.id!==args.id);return null}
 if(name==='model_fetch_available')return ['glm-5.3','glm-5.3-flash','glm-4.6'];
 return [];
})});
afterEach(cleanup);
async function openPanel(){render(<App/>);fireEvent.click(screen.getByRole('button',{name:/模型设置/}));await screen.findByRole('button',{name:'添加自定义供应商'})}

it('内置供应商展示内置徽标与启停开关，启停写入个人覆盖',async()=>{
 await openPanel();
 expect(screen.getAllByRole('checkbox',{name:'启用供应商 DeepSeek'})).toHaveLength(1);
 fireEvent.click(screen.getByRole('checkbox',{name:'启用供应商 DeepSeek'}));
 await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('provider_save',{config:expect.objectContaining({id:'deepseek',enabled:false}),secret:undefined}));
});

it('内置模型默认启用、可停用，信息卡只读并说明不可改删',async()=>{
 await openPanel();
 const deepseekSwitch=screen.getByRole('checkbox',{name:'启用模型 DeepSeek Flash'});
 expect(deepseekSwitch).toBeChecked();
 fireEvent.click(deepseekSwitch);
 await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('provider_save',{config:expect.objectContaining({id:'deepseek',disabledModels:['deepseek-flash']}),secret:undefined}));
 fireEvent.click(screen.getByRole('button',{name:/DeepSeek Flash/}));
 await screen.findByRole('heading',{name:'内置模型：DeepSeek Flash'});
 expect(screen.getByText(/不可修改或删除/)).toBeInTheDocument();
 expect(screen.getByText(/disabled \/ enabled/)).toBeInTheDocument();
});

it('添加模型支持档位多选与默认档位，保存写入新字段',async()=>{
 await openPanel();
 fireEvent.click(screen.getByRole('button',{name:'添加模型'}));
 fireEvent.change(screen.getByLabelText('显示名称'),{target:{value:'推理配置'}});
 fireEvent.change(screen.getByLabelText('模型标识'),{target:{value:'glm-5.3'}});
 fireEvent.click(screen.getByLabelText('档位 low'));
 fireEvent.click(screen.getByLabelText('档位 max'));
 fireEvent.change(screen.getByLabelText('默认推理档位'),{target:{value:'max'}});
 fireEvent.click(screen.getByRole('button',{name:'保存模型'}));
 await screen.findByText('已保存模型。');
 expect(mocks.command).toHaveBeenCalledWith('model_save',{config:expect.objectContaining({providerId:'deepseek',model:'glm-5.3',efforts:['low','max'],effort:'max',enabled:true,builtin:false})});
});

it('从 API 获取模型列表并选择填充模型标识',async()=>{
 await openPanel();
 fireEvent.click(screen.getByRole('button',{name:'添加模型'}));
 fireEvent.change(screen.getByLabelText('显示名称'),{target:{value:'检测'}});
 fireEvent.click(screen.getByRole('button',{name:'从 API 获取模型列表'}));
 const pick=await screen.findByLabelText('从检测结果选择');
 expect(mocks.command).toHaveBeenCalledWith('model_fetch_available',{providerId:'deepseek',apiMode:'chatCompletions'});
 fireEvent.change(pick,{target:{value:'glm-5.3-flash'}});
 expect(screen.getByLabelText('模型标识')).toHaveValue('glm-5.3-flash');
});

it('供应商启停后其模型不可添加，自定义模型可停用',async()=>{
 await openPanel();
 fireEvent.click(screen.getByRole('button',{name:'添加模型'}));
 fireEvent.change(screen.getByLabelText('显示名称'),{target:{value:'m1'}});
 fireEvent.change(screen.getByLabelText('模型标识'),{target:{value:'model-x'}});
 fireEvent.click(screen.getByRole('button',{name:'保存模型'}));
 await screen.findByText('已保存模型。');
 const modelSwitch=screen.getByRole('checkbox',{name:'启用模型 m1'});
 expect(modelSwitch).toBeChecked();
 fireEvent.click(modelSwitch);
 await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('model_save',{config:expect.objectContaining({name:'m1',enabled:false})}));
});

it('编辑已保存模型仅在干净时可测试，删除模型需先保存',async()=>{
 await openPanel();
 fireEvent.click(screen.getByRole('button',{name:'添加模型'}));
 fireEvent.change(screen.getByLabelText('显示名称'),{target:{value:'待测'}});
 fireEvent.change(screen.getByLabelText('模型标识'),{target:{value:'model-y'}});
 expect(screen.getByRole('button',{name:'测试已保存配置'})).toBeDisabled();
 expect(screen.getByRole('button',{name:'删除模型'})).toBeDisabled();
 fireEvent.click(screen.getByRole('button',{name:'保存模型'}));
 await screen.findByText('已保存模型。');
 expect(screen.getByRole('button',{name:'删除模型'})).toBeEnabled();
 fireEvent.click(screen.getByRole('button',{name:'测试已保存配置'}));
 await screen.findByRole('alert');
 expect(screen.getByRole('alert')).toHaveTextContent('服务返回 401');
});

it('复制供应商清空密钥输入且副本端点可编辑',async()=>{
 await openPanel();
 fireEvent.change(screen.getByLabelText('API 密钥'),{target:{value:'unsaved-secret'}});
 fireEvent.click(screen.getByRole('button',{name:'复制供应商'}));
 await screen.findByRole('button',{name:/DeepSeek 副本/});
 expect(screen.getByLabelText('API 密钥')).toHaveValue('');
 expect(screen.getByLabelText('供应商名称')).not.toHaveAttribute('readonly');
});

it('自定义供应商可以只填所需协议并保存模型',async()=>{
 await openPanel();
 fireEvent.click(screen.getByRole('button',{name:'添加自定义供应商'}));
 fireEvent.change(screen.getByLabelText('供应商名称'),{target:{value:'私有供应商'}});
 fireEvent.click(screen.getByRole('button',{name:'添加模型'}));
 fireEvent.change(screen.getByLabelText('API 地址'),{target:{value:'https://private.test/v1'}});
 fireEvent.change(screen.getByLabelText('显示名称'),{target:{value:'私有模型'}});
 fireEvent.change(screen.getByLabelText('模型标识'),{target:{value:'private-model'}});
 fireEvent.click(screen.getByRole('button',{name:'保存供应商'}));
 await screen.findByText('已保存供应商；密钥由系统凭据管理器保管。');
 fireEvent.click(screen.getByRole('button',{name:'保存模型'}));
 await screen.findByText('已保存模型。');
 expect(mocks.command).toHaveBeenCalledWith('provider_save',{config:expect.objectContaining({name:'私有供应商',compatibility:'standard'}),secret:undefined});
 expect(mocks.command).toHaveBeenCalledWith('model_save',{config:expect.objectContaining({model:'private-model',baseUrl:'https://private.test/v1'})});
});

it('关联模型阻止删除供应商，删除模型后才允许移除',async()=>{
 await openPanel();
 fireEvent.click(screen.getByRole('button',{name:'复制供应商'}));
 await screen.findByRole('button',{name:/DeepSeek 副本/});
 fireEvent.click(screen.getByRole('button',{name:'添加模型'}));
 fireEvent.change(screen.getByLabelText('显示名称'),{target:{value:'副本模型'}});
 fireEvent.change(screen.getByLabelText('模型标识'),{target:{value:'copy-model'}});
 fireEvent.click(screen.getByRole('button',{name:'保存模型'}));
 await screen.findByText('已保存模型。');
 expect(screen.getByRole('button',{name:'删除供应商'})).toBeDisabled();
 fireEvent.click(screen.getByRole('button',{name:'删除模型'}));
 await waitFor(()=>expect(screen.getByRole('button',{name:'删除供应商'})).not.toBeDisabled());
 fireEvent.click(screen.getByRole('button',{name:'删除供应商'}));
 await waitFor(()=>expect(screen.queryByRole('button',{name:/DeepSeek 副本/})).not.toBeInTheDocument());
 expect(mocks.command).toHaveBeenCalledWith('provider_remove',{id:'copy'});
});
