import {create} from 'zustand';
export type SideView='changes'|'preview'|'web'|'plans'|'pdf'|'chat';
export interface ChatInitial{sessionId:string;text:string}
interface SidePanelState {open:boolean;view:SideView;previewPath:string|null;webUrl:string|null;pdfPath:string|null;pdfRevision:number;chatSessionId:string|null;chatParentSessionId:string|null;chatDrafts:Record<string,string>;chatInitial:ChatInitial|null;openPdf:(path:string)=>void;openChanges:()=>void;openPreview:(path:string)=>void;openWeb:(url:string)=>void;openPlans:()=>void;openChat:(sessionId:string,parentSessionId:string|null,initialText?:string)=>void;setChatDraft:(sessionId:string,text:string)=>void;consumeChatInitial:(sessionId:string)=>string|null;closeChat:()=>void;setView:(view:SideView)=>void;updateWebUrl:(url:string|null)=>void;toggle:()=>void;close:()=>void;reset:()=>void}
export const useSidePanel=create<SidePanelState>((set,get)=>({
 open:false,view:'changes',previewPath:null,webUrl:null,pdfPath:null,pdfRevision:0,chatSessionId:null,chatParentSessionId:null,chatDrafts:{},chatInitial:null,
 openPdf:(path)=>set(s=>({open:true,view:'pdf',pdfPath:path,pdfRevision:s.pdfRevision+1})),
 openChanges:()=>set({open:true,view:'changes'}),
 openPreview:(path)=>set({open:true,view:'preview',previewPath:path}),
 openWeb:(url)=>set({open:true,view:'web',webUrl:url}),
 openPlans:()=>set({open:true,view:'plans'}),
 // 侧聊：一次只承载一个会话；带首条文字时由侧聊视图在时间线激活后消费，
 // 避免实时事件早于 timeline 建立被 useChat 丢弃。
 openChat:(sessionId,parentSessionId,initialText)=>set({open:true,view:'chat',chatSessionId:sessionId,chatParentSessionId:parentSessionId,chatInitial:initialText?.trim()?{sessionId,text:initialText}:null}),
 setChatDraft:(sessionId,text)=>set(s=>({chatDrafts:{...s.chatDrafts,[sessionId]:text}})),
 consumeChatInitial:(sessionId)=>{const current=get().chatInitial;if(!current||current.sessionId!==sessionId)return null;set({chatInitial:null});return current.text},
 // 关闭侧聊视图：丢弃当前会话绑定，其他视图与已输入草稿保留。
 closeChat:()=>set(s=>({chatSessionId:null,chatParentSessionId:null,chatInitial:null,view:s.view==='chat'?'changes':s.view})),
 setView:(view)=>set({view}),
 // 网页面板导航完成后同步实际地址（地址栏与「网页」tab 存在性）。
 updateWebUrl:(url)=>set({webUrl:url}),
 toggle:()=>set(s=>({open:!s.open})),
 close:()=>set({open:false}),
 reset:()=>set({open:false,view:'changes',previewPath:null,webUrl:null,pdfPath:null,chatSessionId:null,chatParentSessionId:null,chatDrafts:{},chatInitial:null}),
}));
