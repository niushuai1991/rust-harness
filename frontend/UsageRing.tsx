import {useEffect,useMemo,useRef,useState} from 'react';
import {RefreshCw} from 'lucide-react';
import {command,errorText} from './api';
import {contextTokensFromEvents,formatResetTime,formatTokens,USAGE_CARD_LABELS} from './contextUsage';
import {tokenBudgetFromEvents} from './tokenBudget';
import {useChat} from './store';
import type {ModelConfig,ProviderInfo,ProviderUsageSnapshot,UsageCardKind} from './types';

const RING_OK='#768178',RING_WARN='#a06a43',RING_DANGER='#a53e3a';
const CIRCUMFERENCE=2*Math.PI*10;
const DEFAULT_BUDGET_LIMIT=1_000_000;
const EMPTY_EVENTS:never[]=[];

function usedPercent(tokens:number,windowSize:number):number{
 if(windowSize<=0)return 0;
 return Math.min(100,Math.max(0,tokens/windowSize*100));
}
/// 占用口径的紧急度颜色：<70% 灰绿，70–90% 琥珀，≥90% 红（与自动压缩告警语义呼应）。
function ringColor(percent:number):string{
 if(percent>=90)return RING_DANGER;
 if(percent>=70)return RING_WARN;
 return RING_OK;
}
function ringSvg(percent:number):string{
 const color=ringColor(percent),offset=(CIRCUMFERENCE*(1-percent/100)).toFixed(2);
 return `<svg viewBox="0 0 24 24" width="15" height="15" aria-hidden="true">`
  +`<circle cx="12" cy="12" r="10" fill="none" stroke="${color}" stroke-width="4" opacity="0.25"/>`
  +`<circle cx="12" cy="12" r="10" fill="none" stroke="${color}" stroke-width="4" opacity="0.7" stroke-linecap="round" stroke-dasharray="${CIRCUMFERENCE.toFixed(2)}" stroke-dashoffset="${offset}" transform="rotate(-90 12 12)"/>`
  +`</svg>`;
}

/// 会话输入区的用量环形：本体显示当前会话上下文占用；悬停/点击弹出双段明细
/// （上段上下文窗口，下段供应商套餐额度；按量计费的供应商只有上段）。
export function UsageRing({models,providers,modelId,sessionId}:{models:ModelConfig[];providers:ProviderInfo[];modelId:string|null;sessionId:string|null}){
 const events=useChat(state=>sessionId?state.events[sessionId]??EMPTY_EVENTS:EMPTY_EVENTS);
 const tokens=useMemo(()=>contextTokensFromEvents(events),[events]);
 const budget=useMemo(()=>tokenBudgetFromEvents(events),[events]);
 const model=models.find(item=>item.id===modelId)??null;
 const windowSize=model?.contextWindow??null;
 const provider=providers.find(item=>item.id===model?.providerId)??null;
 const usageSupported=!!provider&&provider.builtin&&(provider.compatibility==='bigModel'||provider.auth==='chatGptOAuth');
 const [open,setOpen]=useState(false);
 const [usage,setUsage]=useState<ProviderUsageSnapshot|null>(null);
 const [usageError,setUsageError]=useState('');
 const [usageBusy,setUsageBusy]=useState(false);
 const [budgetLimit,setBudgetLimit]=useState(DEFAULT_BUDGET_LIMIT);
 const [budgetDraft,setBudgetDraft]=useState(String(DEFAULT_BUDGET_LIMIT));
 const [budgetError,setBudgetError]=useState('');
 const [budgetNotice,setBudgetNotice]=useState('');
 const [budgetBusy,setBudgetBusy]=useState(false);
 const timer=useRef<number|null>(null);
 const loadedAt=useRef(0),inflight=useRef(false),alive=useRef(true);
 useEffect(()=>{alive.current=true;return()=>{alive.current=false}},[]);
 useEffect(()=>{
  void command<number>('token_budget_limit_get').then(limit=>{
   if(!alive.current||!Number.isSafeInteger(limit)||limit<=0)return;
   setBudgetLimit(limit);setBudgetDraft(String(limit));
  }).catch(error=>{if(alive.current)setBudgetError(errorText(error))});
 },[]);

 async function saveBudgetLimit(){
  const limit=Number(budgetDraft.trim());
  if(!Number.isSafeInteger(limit)||limit<=0){setBudgetError('请输入正整数 Token 上限');setBudgetNotice('');return}
  setBudgetBusy(true);setBudgetError('');setBudgetNotice('');
  try{
   await command<void>('token_budget_limit_set',{limitTokens:limit});
   setBudgetLimit(limit);setBudgetDraft(String(limit));setBudgetNotice('已保存，从下一条用户消息起生效');
  }catch(error){setBudgetError(errorText(error))}finally{setBudgetBusy(false)}
 }

 async function loadUsage(force=false){
  if(!usageSupported||inflight.current)return;
  if(!force&&loadedAt.current&&Date.now()-loadedAt.current<60_000)return;
  inflight.current=true;
  setUsageBusy(true);setUsageError('');
  try{
   const snapshot=await command<ProviderUsageSnapshot>('provider_usage',{providerId:provider!.id});
   if(!alive.current)return;
   setUsage(snapshot);loadedAt.current=Date.now();
  }catch(error){
   if(!alive.current)return;
   setUsage(null);setUsageError(errorText(error));
  }finally{
   inflight.current=false;
   if(alive.current)setUsageBusy(false);
  }
 }
 useEffect(()=>{
  if(open&&usageSupported)void loadUsage();
  // eslint-disable-next-line react-hooks/exhaustive-deps -- 面板展开时按需拉取；loadUsage 闭包读取最新 props。
 },[open,usageSupported]);
 useEffect(()=>{
  function onDocumentClick(event:MouseEvent){
   if(!wrapRef.current?.contains(event.target as Node))setOpen(false);
  }
  document.addEventListener('click',onDocumentClick);
  return()=>document.removeEventListener('click',onDocumentClick);
 },[]);

 const wrapRef=useRef<HTMLDivElement|null>(null);
 const used=tokens??0;
 const budgetUsed=budget?.usedTokens??0;
 const displayedBudgetLimit=budget?.limitTokens??budgetLimit;
 const budgetPercent=usedPercent(budgetUsed,displayedBudgetLimit);
 const percent=windowSize==null?budgetPercent:usedPercent(used,windowSize);
 const ringLabel=windowSize==null?`本轮预算 ${Math.round(percent)}%`:`上下文用量 ${Math.round(percent)}%`;
 const showCards=usageSupported&&!!usage&&usage.cards.length>0;
 // hover 处理挂在外层容器（覆盖按钮与浮层卡片）：光标移入浮层不会触发关闭定时器。
 return <div className="usage-ring-wrap" ref={wrapRef}
  onMouseEnter={()=>{if(timer.current){window.clearTimeout(timer.current);timer.current=null}setOpen(true)}}
  onMouseLeave={()=>{timer.current=window.setTimeout(()=>{timer.current=null;setOpen(false)},120)}}>
  <button type="button" className="usage-ring-btn" aria-label={ringLabel} aria-haspopup="dialog" aria-expanded={open}
   onClick={()=>{
    if(timer.current){window.clearTimeout(timer.current);timer.current=null}
    setOpen(old=>!old);
   }}>
   <span dangerouslySetInnerHTML={{__html:ringSvg(percent)}}/>
  </button>
  {open&&<div className="usage-ring-pop" role="dialog" aria-label="用量明细">
   {windowSize!=null&&<>
    <p className="usage-ring-title">上下文窗口</p>
    <div className="usage-ring-line"><strong>{formatTokens(used)} / {formatTokens(windowSize)}</strong><span>{Math.round(usedPercent(used,windowSize))}%</span></div>
    <div className="usage-bar usage-ring-bar" role="progressbar" aria-valuenow={Math.round(usedPercent(used,windowSize))} aria-valuemin={0} aria-valuemax={100} aria-label="上下文占用百分比"><i className={usedPercent(used,windowSize)>=70?'warn':''} style={{width:`${usedPercent(used,windowSize)}%`}}/></div>
   </>}
   <p className="usage-ring-title usage-ring-gap">本轮 Token 预算</p>
   <div className="usage-ring-line"><strong>{formatTokens(budgetUsed)} / {formatTokens(displayedBudgetLimit)}</strong><span>{Math.round(budgetPercent)}%</span></div>
   <div className="usage-bar usage-ring-bar" role="progressbar" aria-valuenow={Math.round(budgetPercent)} aria-valuemin={0} aria-valuemax={100} aria-label="本轮 Token 预算占用百分比"><i className={budgetPercent>=90?'warn':''} style={{width:`${budgetPercent}%`}}/></div>
   <form className="usage-budget-setting" onSubmit={event=>{event.preventDefault();void saveBudgetLimit()}}>
    <label htmlFor="token-budget-limit">每回合上限（下一条消息生效）</label>
    <div className="usage-budget-input"><input id="token-budget-limit" type="number" min="1" max={Number.MAX_SAFE_INTEGER} step="1" value={budgetDraft} onChange={event=>setBudgetDraft(event.currentTarget.value)}/><button type="submit" disabled={budgetBusy}>{budgetBusy?'保存中…':'保存'}</button></div>
    {budgetError&&<p className="usage-budget-error" role="alert">{budgetError}</p>}
    {budgetNotice&&<p className="usage-budget-notice" role="status">{budgetNotice}</p>}
   </form>
   {usageSupported&&<p className="usage-ring-title usage-ring-gap">套餐额度{usage?.planName&&<em>{usage.planName}</em>}</p>}
   {usageSupported&&(usageError
    ?<div className="usage-error" role="status">{usageError}<button type="button" onClick={()=>void loadUsage(true)}>重试</button></div>
    :usageBusy&&!usage?<p className="muted usage-ring-hint">正在获取额度…</p>
    :usage&&showCards?usage.cards.map(card=>{
     const pct=card.remainingPercent==null?null:Math.max(0,Math.min(100,Math.round(card.remainingPercent)));
     return <div key={card.kind} className="usage-ring-card">
      <div className="usage-ring-card-top"><span>{USAGE_CARD_LABELS[card.kind]}</span><strong>{pct==null?'—':`${pct}%`}</strong></div>
      <div className="usage-ring-card-bar">
       <div className="usage-ring-track"><i className={pct!=null&&pct<20?'warn':''} style={{width:`${pct??0}%`}}/></div>
       <em className="usage-ring-reset">{formatResetTime(card.resetAt,card.kind)}</em>
      </div>
     </div>;
    })
    :usage&&!usage.cards.length?<p className="muted usage-ring-hint">当前套餐未返回额度明细。</p>:null)}
  </div>}
 </div>;
}
