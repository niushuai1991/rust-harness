import {useEffect,useRef,useState} from 'react';
import {X} from 'lucide-react';
import {useSidePanel} from './sidePanelStore';
import {ChangesView} from './ChangesView';
import {PreviewView} from './PreviewView';
import {WebPanel} from './WebPanel';
const WIDTH_KEY='side-panel-width';
const MIN_WIDTH=280;
function savedWidth(){const value=Number(localStorage.getItem(WIDTH_KEY));return Number.isFinite(value)&&value>=MIN_WIDTH?value:430}
// 右侧面板容器：更改审阅 + 文件预览；宽度手写 pointer 拖拽并记忆（参考 zcode，避免整树重渲染）。
export function SidePanel({workspaceId,executionMode}:{workspaceId:string;executionMode?:'replica'|'worktree'}) {
 const open=useSidePanel(s=>s.open),view=useSidePanel(s=>s.view),previewPath=useSidePanel(s=>s.previewPath),webUrl=useSidePanel(s=>s.webUrl),setView=useSidePanel(s=>s.setView),close=useSidePanel(s=>s.close);
 const [width,setWidth]=useState(savedWidth);
 const widthRef=useRef(width);widthRef.current=width;
 useEffect(()=>{
  if(!open)return;
  const onKey=(e:KeyboardEvent)=>{if(e.key==='Escape')close()};
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
 return <aside className="side-panel" style={{width}} aria-label="文件面板">
  <header className="side-panel-head">
   <div className="side-panel-tabs" role="tablist" aria-label="面板视图">
    <button type="button" role="tab" aria-selected={view==='changes'} onClick={()=>setView('changes')}>更改</button>
    <button type="button" role="tab" aria-selected={view==='preview'} onClick={()=>setView('preview')}>预览</button>
    {webUrl&&<button type="button" role="tab" aria-selected={view==='web'} onClick={()=>setView('web')}>网页</button>}
   </div>
   <button type="button" className="icon-soft" aria-label="关闭面板" onClick={close}><X size={17}/></button>
  </header>
  <div className="side-panel-body">
   {view==='changes'?<ChangesView workspaceId={workspaceId} executionMode={executionMode}/>:view==='preview'?<PreviewView workspaceId={workspaceId} path={previewPath}/>:<WebPanel url={webUrl}/>}
  </div>
  <div className="side-panel-resizer" role="separator" aria-orientation="vertical" aria-label="调整面板宽度" onPointerDown={startDrag}/>
 </aside>;
}
