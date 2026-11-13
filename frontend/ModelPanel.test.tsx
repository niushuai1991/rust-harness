import {afterEach,beforeEach,expect,it,vi} from 'vitest';
import {cleanup,fireEvent,render,screen,waitFor,within} from '@testing-library/react';
import App from './App';
const mocks=vi.hoisted(()=>({command:vi.fn()}));
vi.mock('./api',()=>({desktop:()=>true,command:mocks.command,send:vi.fn(),errorText:(e:unknown)=>e instanceof Error?e.message:String(e)}));
vi.mock('@tauri-apps/plugin-dialog',()=>({open:vi.fn(),save:vi.fn()}));
const baseProvider={enabled:true,builtinModels:[],disabledModels:[]};
const deepseek={...baseProvider,auth:'apiKey',id:'deepseek',name:'DeepSeek',builtin:true,compatibility:'deepSeek',endpoints:[{apiMode:'chatCompletions',baseUrl:'https://api.deepseek.com'},{apiMode:'responses',baseUrl:'https://api.deepseek.com'},{apiMode:'anthropic',baseUrl:'https://api.deepseek.com/anthropic'}],defaultModel:'deepseek-flash',credentialRef:'deep-key',proxy:null,
 builtinModels:[{id:'deepseek-flash',name:'DeepSeek Flash',efforts:['disabled','enabled']},{id:'deepseek-v4-pro',name:'DeepSeek V4 Pro',efforts:['disabled','enabled']}]};
const zhipu={...deepseek,id:'zhipu',name:'智谱 Coding Plan',compatibility:'bigModel',endpoints:[{apiMode:'chatCompletions',baseUrl:'https://open.bigmodel.cn/api/coding/paas/v4'},{apiMode:'responses',baseUrl:'https://open.bigmodel.cn/api/coding/paas/v4'},{apiMode:'anthropic',baseUrl:'https://open.bigmodel.cn/api/anthropic'}],defaultModel:'glm-5.3',credentialRef:'glm-key',
 builtinModels:[{id:'glm-5.3',name:'GLM-5.3',efforts:['low','high','max']}]};
const chatgpt={...baseProvider,auth:'chatGptOAuth',id:'chatgpt',name:'ChatGPT 账号',builtin:true,compatibility:'standard',endpoints:[{apiMode:'responses',baseUrl:'https://chatgpt.com/backend-api/codex'}],defaultModel:'gpt-6.1-sol',credentialRef:'provider/chatgpt',proxy:null,
 builtinModels:[{id:'gpt-6.1-sol',name:'GPT-6.1-Sol',efforts:['low','medium','high','xhigh','max']},{id:'gpt-5.5',name:'GPT-5.5',efforts:['low','medium','high','xhigh']}]};
let catalog:any[]=[],savedModels:any[]=[],usageResult:unknown,accountResult:any,loginStatusQueue:any[];
// 用本地时区构造重置时间，断言在不同时区的 CI 上都稳定。
const weeklyReset=new Date(2026,10,8,10,22).getTime(),toolsReset=new Date(2026,11,1).getTime();
const usageOk={planName:'GLM Coding Pro',cards:[
 {kind:'fiveHour',remainingPercent:70,resetAt:null},
 {kind:'weekly',remainingPercent:14,resetAt:weeklyReset},
 {kind:'tools',remainingPercent:99,resetAt:toolsReset}]};
function reset(){catalog=[structuredClone(deepseek),structuredClone(zhipu),structuredClone(chatgpt)];savedModels=[];usageResult=structuredClone(usageOk);accountResult={loggedIn:false,email:null,planType:null,expiresAt:null};loginStatusQueue=[]}
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
 if(name==='provider_usage'){if(usageResult instanceof Error)throw usageResult;return structuredClone(usageResult)}
 if(name==='chatgpt_account_status')return structuredClone(accountResult);
 if(name==='chatgpt_login_start')return 'http://127.0.0.1:1455/auth/callback?state=s';
 if(name==='chatgpt_login_status')return loginStatusQueue.length?loginStatusQueue.shift():{status:'idle',email:null,planType:null,error:null};
 if(name==='chatgpt_logout'){accountResult={loggedIn:false,email:null,planType:null,expiresAt:null};return null}
 return [];
})});
afterEach(cleanup);
async function openPanel(){render(<App/>);fireEvent.click(screen.getByRole('button',{name:'设置'}));const page=await screen.findByRole('dialog',{name:'设置'});fireEvent.click(within(page).getByRole('button',{name:'模型设置'}));await screen.findByRole('button',{name:'添加自定义供应商'})}
async function chooseZhipu(){fireEvent.click(screen.getByRole('button',{name:/智谱 Coding Plan/}));await screen.findByText('GLM Coding Pro')}

it('自定义模型视觉能力可设置并保存，旧配置默认未知',async()=>{
 await openPanel();
 fireEvent.click(screen.getByRole('button',{name:'添加模型'}));
 expect(screen.getByLabelText('图片输入能力')).toHaveValue('unknown');
 fireEvent.change(screen.getByLabelText('显示名称'),{target:{value:'视觉模型'}});
 fireEvent.change(screen.getByLabelText('模型标识'),{target:{value:'vision'}});
 fireEvent.change(screen.getByLabelText('图片输入能力'),{target:{value:'true'}});
 fireEvent.click(screen.getByRole('button',{name:'保存模型'}));
 await screen.findByText('已保存模型。');
 expect(mocks.command).toHaveBeenCalledWith('model_save',{config:expect.objectContaining({supportsVision:true})});
 fireEvent.click(screen.getByRole('button',{name:'编辑 视觉模型'}));
 expect(screen.getByLabelText('图片输入能力')).toHaveValue('true');
});

it('内置供应商展示内置徽标与启停开关，启停写入个人覆盖',async()=>{
 await openPanel();
 expect(screen.getAllByRole('checkbox',{name:'启用供应商 DeepSeek'})).toHaveLength(1);
 fireEvent.click(screen.getByRole('checkbox',{name:'启用供应商 DeepSeek'}));
 await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('provider_save',{config:expect.objectContaining({id:'deepseek',enabled:false}),secret:undefined}));
});

it('供应商目录按内置与自定义分组渲染',async()=>{
 await openPanel();
 expect(screen.getByRole('heading',{name:'内置供应商'})).toBeInTheDocument();
 expect(screen.getByRole('heading',{name:'自定义供应商'})).toBeInTheDocument();
 const nav=screen.getByRole('navigation',{name:'供应商目录'});
 expect(within(nav).getAllByRole('button',{name:/DeepSeek/})).toHaveLength(1);
});

it('选中智谱 Coding Plan 后自动拉取并渲染剩余额度卡',async()=>{
 await openPanel();
 await chooseZhipu();
 expect(mocks.command).toHaveBeenCalledWith('provider_usage',{providerId:'zhipu'});
 expect(screen.getByText('5 小时剩余')).toBeInTheDocument();
 expect(screen.getByText('70%')).toBeInTheDocument();
 expect(screen.getByText('14%')).toBeInTheDocument();
 expect(screen.getByText('99%')).toBeInTheDocument();
 expect(screen.getByText('11月8日 重置')).toBeInTheDocument();
 expect(screen.getByText('12月1日 重置')).toBeInTheDocument();
});

it('额度查询失败时在额度区内显示错误并可重试',async()=>{
 await openPanel();
 usageResult=new Error('获取额度失败：HTTP 401');
 fireEvent.click(screen.getByRole('button',{name:/智谱 Coding Plan/}));
 await screen.findByText('获取额度失败：HTTP 401');
 usageResult=structuredClone(usageOk);
 fireEvent.click(screen.getByRole('button',{name:'重试'}));
 await screen.findByText('GLM Coding Pro');
 expect(screen.getByText('70%')).toBeInTheDocument();
});

it('DeepSeek 等非智谱供应商不请求额度',async()=>{
 await openPanel();
 await waitFor(()=>expect(screen.getByRole('heading',{name:'DeepSeek'})).toBeInTheDocument());
 expect(mocks.command).not.toHaveBeenCalledWith('provider_usage',expect.anything());
});

it('内置模型默认启用、可停用，弹窗信息只读并说明不可改删',async()=>{
 await openPanel();
 const deepseekSwitch=screen.getByRole('checkbox',{name:'启用模型 DeepSeek Flash'});
 expect(deepseekSwitch).toBeChecked();
 fireEvent.click(deepseekSwitch);
 await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('provider_save',{config:expect.objectContaining({id:'deepseek',disabledModels:['deepseek-flash']}),secret:undefined}));
 fireEvent.click(screen.getByRole('button',{name:'DeepSeek Flash'}));
 const dialog=await screen.findByRole('dialog',{name:'内置模型：DeepSeek Flash'});
 expect(within(dialog).getByText(/不可修改或删除/)).toBeInTheDocument();
 expect(within(dialog).getByText(/disabled \/ enabled/)).toBeInTheDocument();
});

it('添加模型弹窗支持档位多选与默认档位，保存后关闭弹窗',async()=>{
 await openPanel();
 fireEvent.click(screen.getByRole('button',{name:'添加模型'}));
 fireEvent.change(screen.getByLabelText('显示名称'),{target:{value:'推理配置'}});
 fireEvent.change(screen.getByLabelText('模型标识'),{target:{value:'glm-5.3'}});
 fireEvent.click(screen.getByLabelText('档位 low'));
 fireEvent.click(screen.getByLabelText('档位 max'));
 fireEvent.change(screen.getByLabelText('默认推理档位'),{target:{value:'max'}});
 fireEvent.click(screen.getByRole('button',{name:'保存模型'}));
 await screen.findByText('已保存模型。');
 expect(screen.queryByLabelText('显示名称')).not.toBeInTheDocument();
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

it('自定义模型行内可启停，供应商停用后内置模型开关禁用',async()=>{
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

it('编辑已保存模型：保存后弹窗内才可测试连接',async()=>{
 await openPanel();
 fireEvent.click(screen.getByRole('button',{name:'添加模型'}));
 fireEvent.change(screen.getByLabelText('显示名称'),{target:{value:'待测'}});
 fireEvent.change(screen.getByLabelText('模型标识'),{target:{value:'model-y'}});
 expect(screen.getByRole('button',{name:'测试已保存配置'})).toBeDisabled();
 fireEvent.click(screen.getByRole('button',{name:'保存模型'}));
 await screen.findByText('已保存模型。');
 fireEvent.click(screen.getByRole('button',{name:'编辑 待测'}));
 expect(screen.getByRole('button',{name:'测试已保存配置'})).toBeEnabled();
 fireEvent.click(screen.getByRole('button',{name:'测试已保存配置'}));
 await screen.findByRole('alert');
 expect(screen.getByRole('alert')).toHaveTextContent('服务返回 401');
});

it('模型行内删除自定义模型',async()=>{
 await openPanel();
 fireEvent.click(screen.getByRole('button',{name:'添加模型'}));
 fireEvent.change(screen.getByLabelText('显示名称'),{target:{value:'待删'}});
 fireEvent.change(screen.getByLabelText('模型标识'),{target:{value:'model-z'}});
 fireEvent.click(screen.getByRole('button',{name:'保存模型'}));
 await screen.findByText('已保存模型。');
 expect(screen.getByRole('button',{name:'删除模型 待删'})).toBeEnabled();
 fireEvent.click(screen.getByRole('button',{name:'删除模型 待删'}));
 await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('model_remove',{modelId:expect.any(String)}));
 expect(screen.queryByRole('button',{name:'删除模型 待删'})).not.toBeInTheDocument();
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

it('关联模型阻止删除供应商，行内删除模型后才允许移除',async()=>{
 await openPanel();
 fireEvent.click(screen.getByRole('button',{name:'复制供应商'}));
 await screen.findByRole('button',{name:/DeepSeek 副本/});
 fireEvent.click(screen.getByRole('button',{name:'添加模型'}));
 fireEvent.change(screen.getByLabelText('显示名称'),{target:{value:'副本模型'}});
 fireEvent.change(screen.getByLabelText('模型标识'),{target:{value:'copy-model'}});
 fireEvent.click(screen.getByRole('button',{name:'保存模型'}));
 await screen.findByText('已保存模型。');
 expect(screen.getByRole('button',{name:'删除供应商'})).toBeDisabled();
 fireEvent.click(screen.getByRole('button',{name:'删除模型 副本模型'}));
 await waitFor(()=>expect(screen.getByRole('button',{name:'删除供应商'})).not.toBeDisabled());
 fireEvent.click(screen.getByRole('button',{name:'删除供应商'}));
 await waitFor(()=>expect(screen.queryByRole('button',{name:/DeepSeek 副本/})).not.toBeInTheDocument());
 expect(mocks.command).toHaveBeenCalledWith('provider_remove',{id:'copy'});
});

it('ChatGPT 供应商无 API 密钥输入，展示登录入口与内置模型',async()=>{
 await openPanel();
 fireEvent.click(screen.getByRole('button',{name:/ChatGPT 账号/}));
 await screen.findByRole('button',{name:'使用 ChatGPT 账号登录'});
 expect(mocks.command).toHaveBeenCalledWith('chatgpt_account_status');
 expect(screen.queryByLabelText('API 密钥')).not.toBeInTheDocument();
 expect(screen.queryByRole('button',{name:'复制供应商'})).not.toBeInTheDocument();
 expect(screen.getAllByRole('button',{name:/GPT-6.1-Sol/}).length).toBeGreaterThan(0);
});

it('ChatGPT 登录发起后进入等待态并展示兜底链接，可取消',async()=>{
 await openPanel();
 fireEvent.click(screen.getByRole('button',{name:/ChatGPT 账号/}));
 fireEvent.click(await screen.findByRole('button',{name:'使用 ChatGPT 账号登录'}));
 expect(mocks.command).toHaveBeenCalledWith('chatgpt_login_start');
 await screen.findByText(/正在等待浏览器完成登录/);
 expect(screen.getByText(/127\.0\.0\.1:1455/)).toBeInTheDocument();
 fireEvent.click(screen.getByRole('button',{name:'取消登录'}));
 await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('chatgpt_login_cancel'));
});

it('ChatGPT 登录完成后展示账号信息并可登出',async()=>{
 loginStatusQueue.push({status:'completed',email:'dev@example.com',planType:'plus',error:null});
 accountResult={loggedIn:true,email:'dev@example.com',planType:'plus',expiresAt:1900000000};
 await openPanel();
 fireEvent.click(screen.getByRole('button',{name:/ChatGPT 账号/}));
 await screen.findByText('dev@example.com',{},{timeout:4000});
 expect(screen.getByText('plus')).toBeInTheDocument();
 fireEvent.click(screen.getByRole('button',{name:'登出'}));
 await screen.findByRole('button',{name:'使用 ChatGPT 账号登录'});
 expect(mocks.command).toHaveBeenCalledWith('chatgpt_logout');
});

it('ChatGPT 登录失败展示原因并可重试',async()=>{
 loginStatusQueue.push({status:'failed',email:null,planType:null,error:'回调 state 校验失败，请重新发起登录'});
 await openPanel();
 fireEvent.click(screen.getByRole('button',{name:/ChatGPT 账号/}));
 fireEvent.click(await screen.findByRole('button',{name:'使用 ChatGPT 账号登录'}));
 // 登录状态轮询间隔 1.5s，等待窗口需覆盖首个轮询周期。
 await screen.findByText('回调 state 校验失败，请重新发起登录',{},{timeout:4000});
 expect(screen.getByRole('button',{name:'重试登录'})).toBeEnabled();
});

it('ChatGPT 模型弹窗不提供 API 模型列表抓取',async()=>{
 await openPanel();
 fireEvent.click(screen.getByRole('button',{name:/ChatGPT 账号/}));
 await screen.findByRole('button',{name:'使用 ChatGPT 账号登录'});
 fireEvent.click(screen.getByRole('button',{name:'添加模型'}));
 expect(screen.queryByRole('button',{name:'从 API 获取模型列表'})).not.toBeInTheDocument();
});
