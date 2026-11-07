import {create} from 'zustand';
export type SideView='changes'|'preview';
interface SidePanelState {open:boolean;view:SideView;previewPath:string|null;openChanges:()=>void;openPreview:(path:string)=>void;setView:(view:SideView)=>void;toggle:()=>void;close:()=>void;reset:()=>void}
// 右侧面板状态：预览路径由聊天流工具卡触发（深层 EventCard），故独立于 App 局部 state。
export const useSidePanel=create<SidePanelState>((set)=>({
 open:false,view:'changes',previewPath:null,
 openChanges:()=>set({open:true,view:'changes'}),
 openPreview:(path)=>set({open:true,view:'preview',previewPath:path}),
 setView:(view)=>set({view}),
 toggle:()=>set(s=>({open:!s.open})),
 close:()=>set({open:false}),
 reset:()=>set({open:false,view:'changes',previewPath:null}),
}));
