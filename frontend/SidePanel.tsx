import {lazy,Suspense,useEffect,useRef,useState} from 'react';
import {X} from 'lucide-react';
import {useSidePanel} from './sidePanelStore';
import {ChangesView} from './ChangesView';
import {PreviewView} from './PreviewView';
import {WebPanel} from './WebPanel';
import {PlansPanel} from './PlansPanel';
import {SideChatView,type SideChatViewProps} from './SideChatView';
const PdfView=lazy(()=>import('./PdfView').then(module=>({default:module.PdfView})));
import type {ExecutionMode} from './types';
const WIDTH_KEY='side-panel-width';
const MIN_WIDTH=280;
function savedWidth(){const value=Number(localStorage.getItem(WIDTH_KEY));return Number.isFinite(value)&&value>=MIN_WIDTH?value:430}
export type SideChatHostProps=Omit<SideChatViewProps,'sessionId'|'parentSessionId'>;
// 右侧面板容器：更改审阅 + 文件预览 + 侧聊；宽度手写 pointer 拖拽并记忆（避免整树重渲染）。
export function SidePanel({workspaceId,executionMode,currentSessionId='',planRevision=0,onOpenSession=()=>{},sideChat}:{workspaceId:string;executionMode?:ExecutionMode;currentSessionId?:string;planRevision?:number;onOpenSession?:(sessionId:string,workspaceId:string)=>void;sideChat?:SideChatHostProps}) {
 const open=useSidePanel(s=>s.open),view=useSidePanel(s=>s.view),previewPath=useSidePanel(s=>s.previewPath),webUrl=useSidePanel(s=>s.webUrl),pdfPath=useSidePanel(s=>s.pdfPath),pdfRevision=useSidePanel(s=>s.pdfRevision),chatSessionId=useSidePanel(s=>s.chatSessionId),chatParentSessionId=useSidePanel(s=>s.chatParentSessionId),setView=useSidePanel(s=>s.setView),close=useSidePanel(s=>s.close);
 const pdfSession=useRef({revision:pdfRevision,sessionId:currentSessionId});
 if(pdfSession.current.revision!==pdfRevision)pdfSession.current={revision:pdfRevision,sessionId:currentSessionId};
 const [width,setWidth]=useState(savedWidth);
 const widthRef=useRef(width);widthRef.current=width;
 // 侧聊有待处理审批/提问时在 tab 上提示（侧聊不出现在侧栏，不能只靠会话行徽标）。
 const chatAttention=!!chatSessionId&&!!sideChat&&(sideChat.approvals.some(item=>item.sessionId===chatSessionId)||sideChat.questions.some(item=>item.sessionId===chatSessionId));
 useEffect(()=>{
  if(!open)return;
  // 输入框内的 Escape 留给输入框（如关闭补全/取消编辑），不关闭整个面板。
  const onKey=(e:KeyboardEvent)=>{if(e.key!=='Escape'||e.defaultPrevented)return;const target=e.target instanceof Element?e.target:null;if(target&&(target.closest('textarea,input,select')||(target as HTMLElement).isContentEditable))return;close()};
  document.addEventListener('keydown',onKey);
  return()=>document.removeEventListener('keydown',onKey);
 },[open,close]);
 function startDrag(e:React.PointerEvent<HTMLDivElement>) {
  e.preventDefault();
  const handle=e.currentTarget;
  handle.setPointerCapture(e.pointerId);
  const move=(ev:PointerEvent)=>{const max=Math.min(window.innerWidth*0.6,880);setWidth(Math.min(Math.max(MIN_WIDTH,window.innerWidth-ev.clientX),max))};
  const up=()=>{handle.removeEventListener('pointermove',move);handle.removeEventListener('pointerup',up);try{localStorage.setItem(WIDTH_KEY,String(widthRef.current))}catch{/* 隐私模式等场景忽略 */}};
  handle.addEventListener('pointermove',move);
  handle.addEventListener('pointerup',up);
 }
 if(!open)return null;
 return <aside className="side-panel" style={{width}} aria-label={view==='chat'&&sideChat?'侧聊面板':'文件面板'}>
  <header className="side-panel-head">
   <div className="side-panel-tabs" role="tablist" aria-label="面板视图">
    <button type="button" role="tab" aria-selected={view==='changes'} disabled={!workspaceId} title={workspaceId?'':'无项目会话没有关联项目，没有可审阅的变更'} onClick={()=>setView('changes')}>更改</button>
    <button type="button" role="tab" aria-selected={view==='preview'} disabled={!workspaceId} title={workspaceId?'':'无项目会话没有关联项目'} onClick={()=>setView('preview')}>预览</button>
    <button type="button" role="tab" aria-selected={view==='plans'} onClick={()=>setView('plans')}>计划</button>
    {chatSessionId&&sideChat&&<button type="button" role="tab" aria-selected={view==='chat'} onClick={()=>setView('chat')}>侧聊{chatAttention&&<span className="tab-dot" role="img" aria-label="侧聊有待处理事项"/>}</button>}
    {pdfPath&&<button type="button" role="tab" aria-selected={view==='pdf'} onClick={()=>setView('pdf')}>PDF</button>}
    {webUrl&&<button type="button" role="tab" aria-selected={view==='web'} onClick={()=>setView('web')}>网页</button>}
   </div>
   <button type="button" className="icon-soft" aria-label="关闭面板" onClick={close}><X size={17}/></button>
  </header>
  <div className={`side-panel-body${view==='chat'?' is-chat':''}`}>
   {/* 侧聊常驻挂载：切到其他视图只隐藏，切回保留草稿与滚动位置。 */}
   {chatSessionId&&sideChat&&<div className="side-chat-pane" hidden={view!=='chat'}><SideChatView key={chatSessionId} sessionId={chatSessionId} parentSessionId={chatParentSessionId} {...sideChat}/></div>}
   {view!=='chat'&&(view==='changes'?<ChangesView workspaceId={workspaceId} executionMode={executionMode}/>:view==='preview'?<PreviewView workspaceId={workspaceId} path={previewPath}/>:view==='pdf'&&pdfPath?<Suspense fallback={<div className="side-empty">正在加载 PDF…</div>}><PdfView key={pdfRevision} path={pdfPath} sessionId={pdfSession.current.sessionId}/></Suspense>:view==='plans'?<PlansPanel currentSessionId={currentSessionId} revision={planRevision} onOpenSession={onOpenSession}/>:<WebPanel url={webUrl}/>)}
  </div>
  <div className="side-panel-resizer" role="separator" aria-orientation="vertical" aria-label="调整面板宽度" onPointerDown={startDrag}/>
 </aside>;
}
