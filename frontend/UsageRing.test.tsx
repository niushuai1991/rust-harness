import {afterEach,expect,it,vi} from 'vitest';
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

afterEach(()=>{cleanup();useChat.setState({events:{},streams:{},statuses:{}})});

it('模型无上下文窗口值时不渲染',()=>{
 const {container}=renderRing({models:[{id:'m1',name:'自定义',baseUrl:'',model:'m',credentialRef:'',proxy:null,providerId:'custom',compatibility:'standard',auth:'apiKey',apiMode:'chatCompletions',maxOutputTokens:null,efforts:[],effort:null,enabled:true,builtin:false} as any],modelId:'m1'});
 expect(container).toBeEmptyDOMElement();
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

it('占用不足 70% 时环形为默认灰绿',()=>{
 pushEvents([assistant({input_tokens:60_000,output_tokens:2_100})]);
 const {container}=renderRing();
 expect(container.querySelector('svg circle')?.getAttribute('stroke')).toBe('#768178');
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
