import {useEffect,useState} from 'react';
import {FileCode} from 'lucide-react';
import {command,errorText} from './api';
import type {WorkspaceFilePreview} from './types';
export const PREVIEW_MAX_BYTES=32768;
// 右侧面板的文件预览：workspace_file_preview 后端截断 32KiB，二进制文件显示占位。
export function PreviewView({workspaceId,path}:{workspaceId:string;path:string|null}) {
 const [preview,setPreview]=useState<WorkspaceFilePreview|null>(null),[error,setError]=useState(''),[busy,setBusy]=useState(false);
 useEffect(()=>{
  setPreview(null);setError('');
  if(!path||!workspaceId)return;
  let alive=true;setBusy(true);
  command<WorkspaceFilePreview>('workspace_file_preview',{workspaceId,path})
   .then(p=>{if(alive)setPreview(p)})
   .catch(e=>{if(alive)setError(errorText(e))})
   .finally(()=>{if(alive)setBusy(false)});
  return()=>{alive=false};
 },[workspaceId,path]);
 if(!path)return <div className="side-empty"><p className="muted">从聊天流的工具卡点击文件名，即可在这里查看工作副本文件。</p></div>;
 if(error)return <div className="side-empty"><p role="alert" className="inline-error">{error}</p></div>;
 if(busy)return <div className="side-empty"><p className="muted">正在读取文件…</p></div>;
 if(!preview)return null;
 return <>
  <div className="preview-head"><FileCode size={15}/><span title={preview.path}>{preview.path}</span></div>
  {preview.binary
   ?<div className="side-empty"><p className="muted">二进制文件，暂不支持预览（{preview.sizeBytes} 字节）。</p></div>
   :<div className="preview-body"><div className="preview-gutter" aria-hidden="true">{preview.content.split('\n').map((_,index)=><span key={index}>{index+1}</span>)}</div><pre className="preview-text">{preview.content}</pre></div>}
  {preview.truncated&&!preview.binary&&<div className="preview-truncated">仅显示前 {PREVIEW_MAX_BYTES/1024} KiB，文件更大（共 {Math.max(1,Math.round(preview.sizeBytes/1024))} KiB）。</div>}
 </>;
}
