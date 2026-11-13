import {create} from 'zustand';
export type SideView='changes'|'preview'|'web'|'plans';
interface SidePanelState {open:boolean;view:SideView;previewPath:string|null;webUrl:string|null;openChanges:()=>void;openPreview:(path:string)=>void;openWeb:(url:string)=>void;openPlans:()=>void;setView:(view:SideView)=>void;toggle:()=>void;close:()=>void;reset:()=>void}
export const useSidePanel=create<SidePanelState>((set)=>({
 open:false,view:'changes',previewPath:null,webUrl:null,
 openChanges:()=>set({open:true,view:'changes'}),
 openPreview:(path)=>set({open:true,view:'preview',previewPath:path}),
 openWeb:(url)=>set({open:true,view:'web',webUrl:url}),
 openPlans:()=>set({open:true,view:'plans'}),
 setView:(view)=>set({view}),
 toggle:()=>set(s=>({open:!s.open})),
 close:()=>set({open:false}),
 reset:()=>set({open:false,view:'changes',previewPath:null,webUrl:null}),
}));
