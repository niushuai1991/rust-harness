import {useEffect,useRef,useState} from 'react';
import {ArrowDown,ArrowUp,Square,X} from 'lucide-react';
import {listen} from '@tauri-apps/api/event';
import {command,errorText,send} from './api';
import {ConversationEvents} from './ConversationEvents';
import {ApprovalPrompt} from './ApprovalPrompt';
import {QuestionCard} from './QuestionCard';
import {useChat} from './store';
import {useSidePanel} from './sidePanelStore';
import type {ApprovalRequest,AskAnswer,AskRequest,LiveEvent,SessionEvent,SessionInfo,TimelinePage} from './types';

const EMPTY_EVENTS:SessionEvent[]=[];

export interface SideChatViewProps {
 sessionId:string;
 parentSessionId:string|null;
 workspaceId:string;
 modelId:string;
 effort:string|null;
 sessions:SessionInfo[];
 activeIds:Set<string>;
 approvals:ApprovalRequest[];
 questions:AskRequest[];
 onResolveApproval:(id:string,allow:boolean)=>void;
 onResolveQuestion:(id:string,answer:AskAnswer)=>void;
 onOpenSession:(id:string)=>void;
}

/// 右侧抽屉里的侧聊：从 side.boundary 之后的空白时间线开始，独立于主会话收发消息。
/// 时间线加载与发送编排自 App 单会话逻辑独立出来（App 的 loader 绑定 activeSessionId，不能复用）。
export function SideChatView({sessionId,parentSessionId,workspaceId,modelId,effort,sessions,activeIds,approvals,questions,onResolveApproval,onResolveQuestion,onOpenSession}:SideChatViewProps) {
 const events=useChat(s=>s.events[sessionId]??EMPTY_EVENTS);
 const timeline=useChat(s=>s.timelines[sessionId]);
 const stream=useChat(s=>s.streams[sessionId]);
 const draft=useSidePanel(s=>s.chatDrafts[sessionId]??'');
 const setChatDraft=useSidePanel(s=>s.setChatDraft);
 const closeChat=useSidePanel(s=>s.closeChat);
 const consumeChatInitial=useSidePanel(s=>s.consumeChatInitial);
 const [loading,setLoading]=useState(true),[olderLoading,setOlderLoading]=useState(false),[turnActive,setTurnActive]=useState(false),[error,setError]=useState('');
 const [showJumpBottom,setShowJumpBottom]=useState(false);
 const scrollRef=useRef<HTMLElement|null>(null),endRef=useRef<HTMLDivElement|null>(null);
 const autoFollow=useRef(true),requestSeq=useRef(0),olderLock=useRef(false),alive=useRef(true),turnActiveRef=useRef(false),steerBusy=useRef(false);
 const loadLatestRef=useRef<()=>Promise<void>>(async()=>{});
 // 本地发起的回合未结束前也算运行中：与主输入框一致，运行中的输入作为补充指令而不是新消息。
 const running=activeIds.has(sessionId)||turnActive;
 const sessionTitle=sessions.find(item=>item.id===sessionId)?.title??'侧聊';
 const parentTitle=parentSessionId?sessions.find(item=>item.id===parentSessionId)?.title??null:null;
 const sideApproval=approvals.find(item=>item.sessionId===sessionId);
 const sideQuestion=questions.find(item=>item.sessionId===sessionId);
 const askAnswers=buildAskAnswers(events);
 const latestBoundary=[...events].reverse().find(event=>event.kind==='turn.start'||event.kind==='turn.end');
 const activeRunId=latestBoundary?.kind==='turn.start'&&typeof latestBoundary.data.runId==='string'?latestBoundary.data.runId:latestBoundary?.kind==='turn.end'?null:[...(timeline?.turns??[])].reverse().find(turn=>!turn.endSeq)?.runId??null;
 const thinking=running&&!stream&&!sideApproval&&!sideQuestion;

 async function loadLatest(){
  const request=++requestSeq.current;
  // 已有历史页时合并最新页（保留向上翻过的内容）；首次加载用整页替换，游标与 hasOlder 来自页面本身。
  const hadPages=useChat.getState().timelines[sessionId]?.pages.some(page=>page.length)??false;
  useChat.getState().activateTimeline(sessionId);
  if(!hadPages)setLoading(true);
  try{
   const page=await command<TimelinePage>('session_timeline_page',{sessionId});
   if(!alive.current||request!==requestSeq.current)return;
   if(hadPages)useChat.getState().mergeLatestTimelinePages(sessionId,[page]);
   else useChat.getState().replaceTimelinePage(sessionId,page);
  }catch(e){if(alive.current&&request===requestSeq.current)setError(errorText(e))}
  finally{if(alive.current&&request===requestSeq.current)setLoading(false)}
 }
 loadLatestRef.current=loadLatest;
 async function loadOlder(){
  if(olderLock.current)return;const current=useChat.getState().timeline(sessionId);
  if(!current?.hasOlder||!current.olderCursor)return;
  olderLock.current=true;setOlderLoading(true);autoFollow.current=false;
  const before=current.olderCursor,node=scrollRef.current,height=node?.scrollHeight??0,top=node?.scrollTop??0;
  try{
   const page=await command<TimelinePage>('session_timeline_page',{sessionId,beforeSeq:before});
   if(!alive.current||useChat.getState().timeline(sessionId)?.olderCursor!==before)return;
   useChat.getState().prependTimelinePage(sessionId,page);
   requestAnimationFrame(()=>{if(node)node.scrollTop=top+node.scrollHeight-height});
  }catch(e){if(alive.current)setError(errorText(e))}
  finally{olderLock.current=false;if(alive.current)setOlderLoading(false)}
 }
 function receive(event:LiveEvent){useChat.getState().receive(event)}
 async function dispatchText(input:string){
  const value=input.trim();if(!value)return;
  // 提问挂起时发送即文本答复（与主输入框一致），不额外入队消息。
  if(sideQuestion){setError('');setChatDraft(sessionId,'');onResolveQuestion(sideQuestion.id,{text:value,source:'text'});return}
  if(!modelId)return;
  setError('');setChatDraft(sessionId,'');
  if(running||turnActiveRef.current){
   if(steerBusy.current)return;
   if(!activeRunId){setError('当前回合尚未就绪或已结束，请稍后重试。');return}
   steerBusy.current=true;
   try{await command('session_steer',{sessionId,expectedRunId:activeRunId,messageId:crypto.randomUUID(),text:value});if(alive.current)await loadLatest()}
   catch(e){if(alive.current)setError(errorText(e))}
   finally{steerBusy.current=false}
   return;
  }
  turnActiveRef.current=true;setTurnActive(true);autoFollow.current=true;
  try{await send(sessionId,value,modelId,receive,undefined,effort)}
  catch(e){if(alive.current)setError(errorText(e))}
  finally{useChat.getState().clearStream(sessionId);turnActiveRef.current=false;if(alive.current){setTurnActive(false);await loadLatest()}}
 }
 async function cancel(){try{await command('session_cancel',{sessionId})}catch(e){if(alive.current)setError(errorText(e))}finally{if(alive.current)await loadLatest()}}
 function jumpToBottom(){const node=scrollRef.current;if(!node)return;autoFollow.current=true;setShowJumpBottom(false);node.scrollTo({top:node.scrollHeight,behavior:'smooth'})}
 useEffect(()=>{alive.current=true;return()=>{alive.current=false;useChat.getState().unpinTimeline(sessionId)}},[sessionId]);
 useEffect(()=>{useChat.getState().pinTimeline(sessionId)},[sessionId]);
 // 打开/切换侧聊：激活并拉取时间线（从 side.boundary 之后的空白段开始）。
 useEffect(()=>{
  void loadLatestRef.current();
 },[]);
 // /side 携带的首条文字：等模型就绪后再消费；timeline 已在首帧同步激活，实时事件不会丢。
 useEffect(()=>{
  if(!modelId)return;
  const initial=consumeChatInitial(sessionId);
  if(initial)void dispatchText(initial);
 },[modelId,sessionId]);
 // 回合结束补拉：全局广播可能因 Lagged 丢失尾部事件，侧聊不在 App 的 activeSession 补拉范围内。
 useEffect(()=>{
  let disposed=false,stop:(()=>void)|undefined;
  listen<SessionEvent>('session-steering-event',event=>{if(event.payload.sessionId===sessionId&&event.payload.kind==='turn.end')void loadLatestRef.current()}).then(un=>{if(disposed)un();else stop=un}).catch(()=>{});
  return()=>{disposed=true;stop?.()};
 },[sessionId]);
 useEffect(()=>{if(autoFollow.current)endRef.current?.scrollIntoView?.({block:'end'})},[events,stream]);
 return <div className="side-chat">
  <header className="side-chat-head"><div className="side-chat-title"><strong>{sessionTitle}</strong>{parentTitle&&<span className="muted">来自「{parentTitle}」</span>}</div><button type="button" className="icon-soft" aria-label="关闭侧聊" title="关闭侧聊（会话保留，可从登记卡重新打开）" onClick={closeChat}><X size={14}/></button></header>
  {error&&<div role="alert" className="side-chat-error"><span>{error}</span><button type="button" aria-label="关闭错误" onClick={()=>setError('')}><X size={12}/></button></div>}
  <section ref={scrollRef} className="side-chat-scroll" aria-label="侧聊时间线" onScroll={event=>{const node=event.currentTarget;const atBottom=node.scrollHeight-node.scrollTop-node.clientHeight<80;autoFollow.current=atBottom;setShowJumpBottom(!atBottom);if(node.scrollTop<=8)void loadOlder()}}>
   <div className="side-chat-messages">
    {timeline?.hasOlder?<div className="timeline-page-control"><button type="button" onClick={()=>void loadOlder()} disabled={olderLoading}>{olderLoading?'正在加载更早消息…':'加载更早消息'}</button></div>:timeline?.loadedOlder?<div className="timeline-page-control"><small>已到侧聊开头</small></div>:null}
    {!loading&&!events.length&&!stream&&<div className="side-chat-empty"><p>侧聊从空白开始</p><p className="muted">父会话历史仅供模型参考，在这里提问不会打断主会话。</p></div>}
    <ConversationEvents turns={timeline?.turns??[]} events={events} workspaceId={workspaceId} onOpenSession={onOpenSession} activeIds={activeIds} askAnswers={askAnswers}/>
    {thinking&&<div className="message assistant thinking-indicator" role="status" aria-label="正在思考"><div className="message-body"><div className="thinking-line"><span>正在思考</span><span className="thinking-dots" aria-hidden="true"><i/><i/><i/></span></div></div></div>}
    {stream&&<article className="message assistant"><div className="message-body"><div className="stream-tag">正在思考与执行</div>{stream.reasoning&&<details className="reasoning"><summary>思考过程</summary><pre>{stream.reasoning}</pre></details>}<div className="prose">{stream.text||'正在处理…'}<span className="cursor"/></div></div></article>}
    <div ref={endRef}/>
   </div>
  </section>
  {showJumpBottom&&<button type="button" className="jump-bottom side-chat-jump" aria-label="滚动到底部" title="滚动到底部" onClick={jumpToBottom}><ArrowDown size={15}/></button>}
  {sideApproval&&<ApprovalPrompt compact approval={sideApproval} sessionTitle={sessionTitle} onResolve={allow=>onResolveApproval(sideApproval.id,allow)}/>}
  {sideQuestion&&<QuestionCard key={sideQuestion.id} question={sideQuestion} sessionTitle={sessionTitle} onSubmit={answer=>onResolveQuestion(sideQuestion.id,answer)}/>}
  <form className="side-chat-composer" onSubmit={event=>{event.preventDefault();void dispatchText(draft)}}>
   <textarea aria-label="侧聊消息" autoFocus value={draft} placeholder={modelId?'在侧聊中提问…（Enter 发送，Shift+Enter 换行）':'模型未就绪，暂不能发送'} onChange={event=>setChatDraft(sessionId,event.target.value)} onKeyDown={event=>{
    if(event.key==='Escape'){event.stopPropagation();return}
    if(event.key==='Enter'&&!event.shiftKey&&!event.nativeEvent.isComposing){event.preventDefault();void dispatchText(draft)}
   }}/>
   <div className="side-chat-actions"><span className="side-chat-hint">{running?'回合运行中，发送将作为补充指令':'使用当前模型 · Enter 发送'}</span>{running&&<button type="button" className="send" aria-label="取消生成" title="停止侧聊回合" onClick={()=>void cancel()}><Square size={14}/></button>}<button type="submit" className="send" aria-label="发送侧聊消息" disabled={!draft.trim()||(!modelId&&!sideQuestion)}><ArrowUp size={16}/></button></div>
  </form>
 </div>;
}

function buildAskAnswers(events:SessionEvent[]){const map:Record<string,{answer:string;source:string}>={};for(const event of events)if(event.kind==='ask.resolved')map[String(event.data.id)]={answer:String(event.data.answer??''),source:String(event.data.source??'')};return map}
