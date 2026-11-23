import {create} from 'zustand';
export type SideView='changes'|'preview'|'web'|'plans'|'pdf';
interface SidePanelState {open:boolean;view:SideView;previewPath:string|null;webUrl:string|null;pdfPath:string|null;pdfRevision:number;openPdf:(path:string)=>void;openChanges:()=>void;openPreview:(path:string)=>void;openWeb:(url:string)=>void;openPlans:()=>void;setView:(view:SideView)=>void;updateWebUrl:(url:string|null)=>void;toggle:()=>void;close:()=>void;reset:()=>void}
export const useSidePanel=create<SidePanelState>((set)=>({
 open:false,view:'changes',previewPath:null,webUrl:null,pdfPath:null,pdfRevision:0,
 openPdf:(path)=>set(s=>({open:true,view:'pdf',pdfPath:path,pdfRevision:s.pdfRevision+1})),
 openChanges:()=>set({open:true,view:'changes'}),
 openPreview:(path)=>set({open:true,view:'preview',previewPath:path}),
 openWeb:(url)=>set({open:true,view:'web',webUrl:url}),
 openPlans:()=>set({open:true,view:'plans'}),
 setView:(view)=>set({view}),
 // 网页面板导航完成后同步实际地址（地址栏与「网页」tab 存在性）。
 updateWebUrl:(url)=>set({webUrl:url}),
 toggle:()=>set(s=>({open:!s.open})),
 close:()=>set({open:false}),
 reset:()=>set({open:false,view:'changes',previewPath:null,webUrl:null,pdfPath:null}),
}));
