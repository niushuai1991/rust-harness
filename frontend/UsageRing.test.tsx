import {afterEach,beforeEach,expect,it,vi} from 'vitest';
import {act,cleanup,fireEvent,render,screen,waitFor} from '@testing-library/react';
import {UsageRing} from './UsageRing';
import {useChat} from './store';
import type {SessionEvent} from './types';

const mocks=vi.hoisted(()=>({command:vi.fn()}));
vi.mock('./api',()=>({desktop:()=>true,command:mocks.command,errorText:(e:unknown)=>e instanceof Error?e.message:String(e)}));

let seq=0;
function event(kind:string,data:Record<string,unknown>):SessionEvent{
 seq+=1;
 return {id:`e${seq}`,sessionId:'s1',seq:String(seq),kind,data,createdAt:'2026-11-08T00:00:00Z'};
}
function assistant(usage:unknown):SessionEvent{return event('assistant.message',{content:'ok',usage})}

const bigmodel={id:'bigmodel',name:'智谱 Coding Plan',builtin:true,compatibility:'bigModel',auth:'apiKey',endpoints:[],defaultModel:'glm-5.3',credentialRef:'glm',proxy:null,enabled:true,builtinModels:[{id:'glm-5.3',name:'GLM-5.3',efforts:[],contextWindow:200_000}],disabledModels:[]} as any;
const deepseek={...bigmodel,id:'deepseek',name:'DeepSeek',compatibility:'deepSeek',credentialRef:'ds',defaultModel:'deepseek-flash',builtinModels:[{id:'deepseek-flash',name:'DeepSeek Flash',efforts:[],contextWindow:128_000}]} as any;
const usageOk={planName:'GLM Coding Pro',cards:[
 {kind:'fiveHour',remainingPercent:70,resetAt:null},
 {kind:'weekly',remainingPercent:14,resetAt:null},
 {kind:'tools',remainingPercent:99,resetAt:null}]};
// UsageRing 的 models 参数是 ModelConfig[]（内置条目由目录合成、contextWindow 已透传）。
const glmConfig={id:'builtin:bigmodel:glm-5.3',name:'GLM-5.3',baseUrl:'https://open.bigmodel.cn/api/coding/paas/v4',model:'glm-5.3',credentialRef:'glm',proxy:null,providerId:'bigmodel',compatibility:'bigModel',auth:'apiKey',apiMode:'chatCompletions',maxOutputTokens:null,contextWindow:200_000,efforts:[],effort:null,enabled:true,builtin:true} as any;
const deepseekConfig={...glmConfig,id:'builtin:deepseek:deepseek-flash',name:'DeepSeek Flash',model:'deepseek-flash',providerId:'deepseek',compatibility:'deepSeek',contextWindow:128_000} as any;

function renderRing(props:{models?:any[];providers?:any[];modelId?:string|null;sessionId?:string|null}={}){
 return render(<UsageRing models={props.models??[glmConfig]} providers={props.providers??[bigmodel]} modelId={props.modelId!==undefined?props.modelId:glmConfig.id} sessionId={props.sessionId!==undefined?props.sessionId:'s1'}/>);
}
function pushEvents(events:SessionEvent[]){act(()=>{useChat.getState().replace('s1',events)})}

beforeEach(()=>{mocks.command.mockReset();mocks.command.mockResolvedValue(undefined)});
afterEach(()=>{cleanup();mocks.command.mockReset();useChat.setState({events:{},streams:{},statuses:{}})});

it('模型没有上下文窗口时仍显示 Token 预算入口',async()=>{
 renderRing({models:[{id:'m1',name:'自定义',baseUrl:'',model:'m',credentialRef:'',proxy:null,providerId:'custom',compatibility:'standard',auth:'apiKey',apiMode:'chatCompletions',maxOutputTokens:null,efforts:[],effort:null,enabled:true,builtin:false} as any],modelId:'m1'});
 const button=screen.getByRole('button',{name:'本轮预算 0%'});
 fireEvent.mouseEnter(button);
 expect(await screen.findByText('本轮 Token 预算')).toBeInTheDocument();
 expect(screen.getByLabelText('每回合上限（下一条消息生效）')).toBeInTheDocument();
});

it('显示本轮累计预算并保存下一回合上限',async()=>{
 pushEvents([event('token.budget',{turnId:'turn-1',usedTokens:12_345,limitTokens:1_000_000,status:'active'})]);
 mocks.command.mockImplementation(async(name:string)=>name==='token_budget_limit_get'?1_000_000:undefined);
 renderRing();
 const button=screen.getByRole('button',{name:/上下文用量/});
 fireEvent.mouseEnter(button);
 expect(await screen.findByText('本轮 Token 预算')).toBeInTheDocument();
 expect(screen.getByText('12.3k / 1M')).toBeInTheDocument();
 const input=screen.getByLabelText('每回合上限（下一条消息生效）');
 fireEvent.change(input,{target:{value:'2000000'}});
 fireEvent.click(screen.getByRole('button',{name:'保存'}));
 expect(await screen.findByRole('status')).toHaveTextContent('已保存，从下一条用户消息起生效');
 expect(mocks.command).toHaveBeenCalledWith('token_budget_limit_set',{limitTokens:2_000_000});
});

it('环形显示上下文占用百分比（usage 归一化后 62.1k/200k ≈ 31%）',()=>{
 pushEvents([assistant({input_tokens:60_000,output_tokens:2_100})]);
 renderRing();
 expect(screen.getByRole('button',{name:'上下文用量 31%'})).toBeInTheDocument();
});

it('面板内的上下文进度条宽度等于占用百分比',async()=>{
 pushEvents([assistant({input_tokens:60_000,output_tokens:2_100})]);
 mocks.command.mockReset();
 mocks.command.mockImplementation(async()=>structuredClone(usageOk));
 const {container}=renderRing();
 fireEvent.mouseEnter(screen.getByRole('button',{name:/上下文用量/}));
 expect(await screen.findByText('上下文窗口')).toBeInTheDocument();
 const progress=container.querySelector('.usage-ring-bar i') as HTMLElement;
 expect(progress.style.width).toBe('31.05%');
});

it('占用达到 90% 时环形转红',()=>{
 pushEvents([assistant({input_tokens:190_000,output_tokens:100})]);
 const {container}=renderRing();
 expect(container.querySelector('svg circle')?.getAttribute('stroke')).toBe('#a53e3a');
});

it('占用不足 70% 时环形使用主题次要文字颜色',()=>{
 pushEvents([assistant({input_tokens:60_000,output_tokens:2_100})]);
 const {container}=renderRing();
 expect(container.querySelector('svg circle')?.getAttribute('stroke')).toBe('var(--muted)');
});

it('悬停弹出双段面板：上下文段 + 智谱套餐三卡',async()=>{
 pushEvents([assistant({input_tokens:60_000,output_tokens:2_100})]);
 mocks.command.mockReset();
 mocks.command.mockImplementation(async(name:string)=>{
  if(name==='provider_usage')return structuredClone(usageOk);
  return [];
 });
 renderRing();
 fireEvent.mouseEnter(screen.getByRole('button',{name:/上下文用量/}));
 expect(await screen.findByText('上下文窗口')).toBeInTheDocument();
 expect(screen.getByText('套餐额度')).toBeInTheDocument();
 expect(await screen.findByText('GLM Coding Pro')).toBeInTheDocument();
 expect(screen.getByText('5 小时剩余')).toBeInTheDocument();
 expect(screen.getByText('每周剩余')).toBeInTheDocument();
 expect(screen.getByText('工具调用')).toBeInTheDocument();
 expect(mocks.command).toHaveBeenCalledWith('provider_usage',{providerId:'bigmodel'});
});

it('按量计费供应商（DeepSeek）面板只有上下文段',async()=>{
 pushEvents([assistant({prompt_tokens:1000,completion_tokens:100})]);
 mocks.command.mockReset();
 mocks.command.mockImplementation(async()=>[]);
 renderRing({providers:[deepseek],models:[deepseekConfig],modelId:deepseekConfig.id});
 fireEvent.mouseEnter(screen.getByRole('button',{name:/上下文用量/}));
 expect(await screen.findByText('上下文窗口')).toBeInTheDocument();
 expect(screen.queryByText('套餐额度')).not.toBeInTheDocument();
 expect(mocks.command).not.toHaveBeenCalledWith('provider_usage',expect.anything());
});

it('额度接口报错时显示错误与重试按钮',async()=>{
 pushEvents([]);
 mocks.command.mockReset();
 mocks.command.mockImplementation(async(name:string)=>{
  if(name==='provider_usage')throw new Error('登录已过期');
  return [];
 });
 renderRing();
 fireEvent.mouseEnter(screen.getByRole('button',{name:/上下文用量/}));
 expect(await screen.findByRole('status')).toHaveTextContent('登录已过期');
 expect(screen.getByRole('button',{name:'重试'})).toBeInTheDocument();
});

it('点击环形按钮可关闭面板',async()=>{
 pushEvents([]);
 mocks.command.mockReset();
 mocks.command.mockImplementation(async()=>structuredClone(usageOk));
 renderRing();
 const button=screen.getByRole('button',{name:/上下文用量/});
 fireEvent.mouseEnter(button);
 await waitFor(()=>expect(screen.getByText('上下文窗口')).toBeInTheDocument());
 fireEvent.click(button);
 await waitFor(()=>expect(screen.queryByText('上下文窗口')).not.toBeInTheDocument());
});

it('光标从按钮移入浮层卡片后浮层保持打开',async()=>{
 pushEvents([]);
 mocks.command.mockReset();
 mocks.command.mockImplementation(async()=>structuredClone(usageOk));
 renderRing();
 const button=screen.getByRole('button',{name:/上下文用量/});
 fireEvent.mouseEnter(button);
 await waitFor(()=>expect(screen.getByRole('dialog',{name:'用量明细'})).toBeInTheDocument());
 // 光标离开按钮（启动 120ms 关闭定时器）后移入浮层卡片，浮层不应关闭。
 fireEvent.mouseLeave(button);
 fireEvent.mouseEnter(screen.getByRole('dialog',{name:'用量明细'}));
 await act(async()=>{await new Promise(resolve=>setTimeout(resolve,150))});
 expect(screen.getByRole('dialog',{name:'用量明细'})).toBeInTheDocument();
});

it('光标移出浮层卡片后浮层延迟自动关闭',async()=>{
 pushEvents([]);
 mocks.command.mockReset();
 mocks.command.mockImplementation(async()=>structuredClone(usageOk));
 renderRing();
 const button=screen.getByRole('button',{name:/上下文用量/});
 fireEvent.mouseEnter(button);
 await waitFor(()=>expect(screen.getByRole('dialog',{name:'用量明细'})).toBeInTheDocument());
 fireEvent.mouseLeave(screen.getByRole('dialog',{name:'用量明细'}));
 await act(async()=>{await new Promise(resolve=>setTimeout(resolve,150))});
 expect(screen.queryByRole('dialog',{name:'用量明细'})).not.toBeInTheDocument();
});

// —— 供应商切换同步：切换模型后套餐数据必须失效并重拉当前供应商 ——

const chatgpt={...bigmodel,id:'chatgpt',name:'ChatGPT',compatibility:'standard',auth:'chatGptOAuth',credentialRef:'',defaultModel:'gpt-6.1-terra',builtinModels:[{id:'gpt-6.1-terra',name:'GPT-6.1 Terra',efforts:[],contextWindow:272_000}]} as any;
const codexConfig={...glmConfig,id:'builtin:chatgpt:gpt-6.1-terra',name:'GPT-6.1 Terra',model:'gpt-6.1-terra',providerId:'chatgpt',compatibility:'standard',auth:'chatGptOAuth',contextWindow:272_000} as any;
const chatgptUsage={planName:'ChatGPT Plus',cards:[{kind:'fiveHour',remainingPercent:50,resetAt:null},{kind:'weekly',remainingPercent:60,resetAt:null}]};

function mockCommand(usage:unknown){
 mocks.command.mockImplementation(async(name:string)=>name==='provider_usage'
  ?structuredClone(usage)
  :name==='token_budget_limit_get'?1_000_000:undefined);
}
async function closePanel(button:HTMLElement){
 fireEvent.mouseLeave(button);
 await act(async()=>{await new Promise(resolve=>setTimeout(resolve,150))});
}
const usageCalls=()=>mocks.command.mock.calls.filter(([name])=>name==='provider_usage').map(([,args])=>(args as {providerId:string}).providerId);

it('切换供应商后重新拉取新供应商套餐',async()=>{
 pushEvents([]);
 mockCommand(usageOk);
 const view=renderRing({providers:[bigmodel,chatgpt],models:[glmConfig,codexConfig]});
 const button=screen.getByRole('button',{name:/上下文用量/});
 fireEvent.mouseEnter(button);
 expect(await screen.findByText('GLM Coding Pro')).toBeInTheDocument();
 await closePanel(button);
 view.rerender(<UsageRing models={[glmConfig,codexConfig]} providers={[bigmodel,chatgpt]} modelId={codexConfig.id} sessionId='s1'/>);
 mockCommand(chatgptUsage);
 fireEvent.mouseEnter(button);
 expect(await screen.findByText('ChatGPT Plus')).toBeInTheDocument();
 expect(screen.queryByText('GLM Coding Pro')).not.toBeInTheDocument();
 expect(usageCalls()).toEqual(['bigmodel','chatgpt']);
});

it('切到按量计费供应商再切回后重新拉取原供应商套餐',async()=>{
 pushEvents([]);
 mockCommand(usageOk);
 const view=renderRing({providers:[bigmodel,deepseek],models:[glmConfig,deepseekConfig]});
 const button=screen.getByRole('button',{name:/上下文用量/});
 fireEvent.mouseEnter(button);
 expect(await screen.findByText('GLM Coding Pro')).toBeInTheDocument();
 await closePanel(button);
 view.rerender(<UsageRing models={[glmConfig,deepseekConfig]} providers={[bigmodel,deepseek]} modelId={deepseekConfig.id} sessionId='s1'/>);
 fireEvent.mouseEnter(button);
 expect(await screen.findByText('上下文窗口')).toBeInTheDocument();
 expect(screen.queryByText('套餐额度')).not.toBeInTheDocument();
 await closePanel(button);
 view.rerender(<UsageRing models={[glmConfig,deepseekConfig]} providers={[bigmodel,deepseek]} modelId={glmConfig.id} sessionId='s1'/>);
 fireEvent.mouseEnter(button);
 expect(await screen.findByText('GLM Coding Pro')).toBeInTheDocument();
 expect(usageCalls()).toEqual(['bigmodel','bigmodel']);
});

it('面板展开时切换供应商立即清空旧套餐数据',async()=>{
 pushEvents([]);
 mockCommand(usageOk);
 const view=renderRing({providers:[bigmodel,chatgpt],models:[glmConfig,codexConfig]});
 const button=screen.getByRole('button',{name:/上下文用量/});
 fireEvent.mouseEnter(button);
 expect(await screen.findByText('GLM Coding Pro')).toBeInTheDocument();
 view.rerender(<UsageRing models={[glmConfig,codexConfig]} providers={[bigmodel,chatgpt]} modelId={codexConfig.id} sessionId='s1'/>);
 expect(screen.queryByText('GLM Coding Pro')).not.toBeInTheDocument();
 await waitFor(()=>expect(usageCalls()).toEqual(["bigmodel","chatgpt"]));
});
