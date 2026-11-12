import {useEffect,useRef,useState,type ReactNode} from 'react';
import {ArrowRight,Archive,Bookmark,Check,ChevronRight,Cpu,Folder,KeyRound,Layers,Plug,Settings,ShieldCheck,Sparkles,Terminal,Undo2,X,type LucideIcon} from 'lucide-react';
import {relativeLabel} from './sessionList';
import type {SessionInfo} from './types';

export type SettingsPanelId='presets'|'skills'|'plugins'|'models';
type SettingsSectionId='overview'|SettingsPanelId|'archive';

type SettingsItem={id:SettingsPanelId|'archive';title:string;description:string;summary:string;group:string;icon:LucideIcon;legacyPanel?:SettingsPanelId};
const ITEMS:SettingsItem[]=[
 {id:'presets',title:'能力预设',description:'组合模型、推理档位与审批策略，快速切换常用工作方式。',summary:'3 个示例预设',group:'工作方式',icon:Bookmark,legacyPanel:'presets'},
 {id:'skills',title:'技能',description:'管理项目与用户技能，让助手按需加载专用工作指引。',summary:'2 个示例技能',group:'工作方式',icon:Sparkles,legacyPanel:'skills'},
 {id:'archive',title:'存档管理',description:'集中查看各项目的已归档会话，随时恢复到任务列表。',summary:'按项目分组 · 可恢复',group:'会话',icon:Archive},
 {id:'plugins',title:'插件与权限',description:'查看扩展、连接与权限策略，了解每项能力的授权范围。',summary:'扩展与安全策略',group:'扩展与安全',icon:Plug,legacyPanel:'plugins'},
 {id:'models',title:'模型设置',description:'管理模型供应商、API 连接与可用模型。',summary:'2 家供应商 · 4 个示例模型',group:'扩展与安全',icon:Cpu,legacyPanel:'models'},
];
const GROUPS=['工作方式','会话','扩展与安全'];
const INTRO:Record<SettingsPanelId|'archive',string>={
 presets:'把常用模型、推理档位和审批方式保存为一组配置，之后可以在发送框快速切换。',
 skills:'查看当前工作区和用户目录中的技能，并管理它们是否参与助手的技能目录。',
 plugins:'集中查看本地插件、MCP 连接及其声明与授权的权限范围。',
 models:'管理模型供应商、凭据和模型目录；API 密钥只保存在系统凭据管理器中。',
 archive:'归档后的会话不会出现在任务列表里，但仍保留在本机；在这里按项目查看并恢复。',
};

export function SettingsPage({onClose,onOpenPanel,archivedSessions,projectName,onRestore}:{onClose:()=>void;onOpenPanel:(panel:SettingsPanelId)=>void;archivedSessions:SessionInfo[];projectName:(id:string)=>string;onRestore:(id:string)=>void}){
 const [active,setActive]=useState<SettingsSectionId>('overview');
 const dialogRef=useRef<HTMLDivElement>(null),closeRef=useRef(onClose);
 closeRef.current=onClose;
 useEffect(()=>{
  const previous=document.activeElement as HTMLElement|null;
  const first=dialogRef.current?.querySelector<HTMLElement>('[data-settings-initial-focus]');
  first?.focus();
  const onKeyDown=(event:KeyboardEvent)=>{
   if(event.key==='Escape'){event.preventDefault();closeRef.current();return}
   if(event.key!=='Tab')return;
   const elements=[...(dialogRef.current?.querySelectorAll<HTMLElement>('button:not(:disabled),a[href],input:not(:disabled),select:not(:disabled),textarea:not(:disabled),[tabindex="0"]')??[])];
   const firstElement=elements[0],lastElement=elements.at(-1);
   if(!firstElement){event.preventDefault();return}
   if(event.shiftKey&&(document.activeElement===firstElement||document.activeElement===dialogRef.current)){event.preventDefault();lastElement?.focus()}
   else if(!event.shiftKey&&(document.activeElement===lastElement||document.activeElement===dialogRef.current)){event.preventDefault();firstElement.focus()}
  };
  document.addEventListener('keydown',onKeyDown);
  return()=>{document.removeEventListener('keydown',onKeyDown);previous?.focus()};
 },[]);
 const openPanel=(panel:SettingsPanelId)=>onOpenPanel(panel);
 const selected=active==='overview'?null:ITEMS.find(item=>item.id===active)!;
 return <div ref={dialogRef} className="settings-screen" role="dialog" aria-modal="true" aria-label="设置" tabIndex={-1}>
  <aside className="settings-sidebar">
   <div className="settings-brand"><span className="settings-brand-mark"><Terminal size={19}/></span><span>Rust Harness<small>偏好与配置</small></span></div>
   <nav className="settings-nav" aria-label="设置导航">
    <button type="button" className={`settings-nav-item${active==='overview'?' active':''}`} aria-current={active==='overview'?'page':undefined} data-settings-initial-focus onClick={()=>setActive('overview')}><Settings size={17}/><span>设置概览</span></button>
    {GROUPS.map(group=><div className="settings-nav-group" key={group}>
     <div className="settings-nav-heading">{group}</div>
     {ITEMS.filter(item=>item.group===group).map(({id,title,icon:Icon})=><button type="button" key={id} className={`settings-nav-item${active===id?' active':''}`} aria-current={active===id?'page':undefined} onClick={()=>setActive(id)}><Icon size={17}/><span>{title}</span>{active===id&&<ChevronRight size={14} className="settings-nav-chevron"/>}</button>)}
    </div>)}
   </nav>
   <div className="settings-sidebar-foot"><div className="settings-local-note"><ShieldCheck size={16}/><span><strong>本地优先</strong><small>配置仅保存在本机</small></span></div><button type="button" className="settings-back-button" onClick={onClose}><ArrowRight size={16}/><span>返回工作台</span></button></div>
  </aside>
  <section className="settings-main" aria-label="设置内容">
   <header className="settings-topbar"><div className="settings-breadcrumb"><span>设置</span>{selected&&<><ChevronRight size={14}/><strong>{selected.title}</strong></>}</div><div className="settings-top-actions"><span className="settings-preview-badge"><Sparkles size={13}/>布局预览</span><button type="button" className="settings-close" aria-label="关闭设置，返回工作台" title="关闭设置" onClick={onClose}><X size={18}/></button></div></header>
   <div className="settings-scroll">
    {selected?<SettingsCategory item={selected} onOpenPanel={openPanel} archivedSessions={archivedSessions} projectName={projectName} onRestore={onRestore}/>:<SettingsOverview onSelect={setActive}/>}
   </div>
  </section>
 </div>;
}

function SettingsOverview({onSelect}:{onSelect:(section:SettingsPanelId|'archive')=>void}){
 return <div className="settings-content">
  <div className="settings-eyebrow">PREFERENCES</div>
  <h1>设置</h1>
  <p className="settings-lead">管理 Rust Harness 的工作方式、技能、扩展权限和模型连接。</p>
  <div className="settings-overview-banner"><div className="settings-banner-icon"><Layers size={20}/></div><div><strong>把常用配置集中在一起</strong><p>选择左侧分类，或从下方卡片进入。页面会覆盖工作台，返回后继续当前会话。</p></div><span className="settings-banner-sparkle"><Sparkles size={18}/></span></div>
  <div className="settings-card-grid" aria-label="设置分类">
   {ITEMS.map(({id,title,description,summary,icon:Icon})=><button type="button" className="settings-category-card" key={id} onClick={()=>onSelect(id)}>
    <span className="settings-card-icon"><Icon size={18}/></span><span className="settings-card-arrow"><ArrowRight size={16}/></span><strong>{title}</strong><span className="settings-card-description">{description}</span><span className="settings-card-summary"><Check size={13}/>{summary}</span>
   </button>)}
  </div>
  <p className="settings-preview-footnote"><span>i</span> 当前为布局预览，示例数据仅用于展示页面结构。</p>
 </div>;
}

function SettingsCategory({item,onOpenPanel,archivedSessions,projectName,onRestore}:{item:SettingsItem;onOpenPanel:(panel:SettingsPanelId)=>void;archivedSessions:SessionInfo[];projectName:(id:string)=>string;onRestore:(id:string)=>void}){
 const Icon=item.icon;
 return <div className="settings-content settings-category-content">
  <div className="settings-eyebrow">{item.group.toUpperCase()}</div>
  <div className="settings-category-heading"><div><h1>{item.title}</h1><p className="settings-lead">{INTRO[item.id]}</p></div><span className="settings-category-icon"><Icon size={21}/></span></div>
  {item.id!=='archive'&&<div className="settings-preview-notice"><Sparkles size={15}/><span>页面布局预览 · 下方为示例内容，不会修改真实设置</span></div>}
  {item.id==='presets'&&<PresetPreview/>}
  {item.id==='skills'&&<SkillsPreview/>}
  {item.id==='archive'&&<ArchivePreview archived={archivedSessions} projectName={projectName} onRestore={onRestore}/>}
  {item.id==='plugins'&&<PluginsPreview/>}
  {item.id==='models'&&<ModelsPreview/>}
  {item.legacyPanel&&<div className="settings-existing-action"><div><strong>继续使用当前管理功能</strong><p>此版先展示设置页的导航与布局；原有配置面板暂时保持不变。</p></div><button type="button" className="settings-manage-button" onClick={()=>onOpenPanel(item.legacyPanel!)}>打开现有面板<ArrowRight size={15}/></button></div>}
 </div>;
}

function PresetPreview(){
 return <div className="settings-example-list">
  <PreviewItem icon={<Bookmark size={16}/>} title="默认（无覆盖）" description="沿用发送框当前选择" badge="内置" selected/>
  <PreviewItem icon={<Terminal size={16}/>} title="快速开发" description="DeepSeek V4 Pro · high · 自动编辑" badge="常用"/>
  <PreviewItem icon={<ShieldCheck size={16}/>} title="谨慎审阅" description="GLM-5.3 · max · 每次确认" badge="自定义"/>
 </div>;
}
function SkillsPreview(){
 return <div className="settings-example-list">
  <PreviewItem icon={<Sparkles size={16}/>} title="$rust-review" description="按项目规范检查 Rust 代码并补充测试" badge="项目 · 已启用"/>
  <PreviewItem icon={<Sparkles size={16}/>} title="$personal-ledger" description="记账项目维护约定与开发流程" badge="用户 · 已启用"/>
  <div className="settings-path-note"><code>.agents/skills/&lt;名称&gt;/SKILL.md</code><span>项目技能目录</span></div>
 </div>;
}
function PluginsPreview(){
 return <>
  <div className="settings-permission-preview"><div className="settings-preview-heading"><span className="settings-card-icon"><ShieldCheck size={17}/></span><div><strong>默认权限模式</strong><small>示例：每次确认</small></div><span className="settings-secure-pill">安全边界保持开启</span></div><p>无论选择哪种模式，模型修改仍先进入工作副本；写回原项目前需要审阅确认。</p><div className="settings-mode-options">{['只读','每次确认','自动编辑','自主','完全访问'].map((name,index)=><span key={name} className={index===1?'selected':''}>{name}</span>)}</div></div>
  <div className="settings-example-list settings-plugin-examples"><PreviewItem icon={<Plug size={16}/>} title="本地插件" description="声明权限与授权范围相交后才可使用" badge="已启用"/><PreviewItem icon={<KeyRound size={16}/>} title="MCP 连接" description="按工作区管理连接状态与授权" badge="按需连接"/></div>
 </>;
}
function ModelsPreview(){
 return <div className="settings-model-preview">
  <div className="settings-provider-card"><div className="settings-provider-heading"><span className="settings-provider-avatar">D</span><div><strong>DeepSeek</strong><small>Anthropic API · 已连接</small></div><span className="settings-provider-status">● 已启用</span></div><div className="settings-model-row"><span><strong>DeepSeek V4 Pro</strong><small>推理档位：disabled / enabled</small></span><span className="settings-model-chip">内置模型</span></div><div className="settings-model-row"><span><strong>deepseek-chat</strong><small>自定义模型 · Chat Completions</small></span><span className="settings-model-chip muted">自定义</span></div></div>
  <div className="settings-provider-card"><div className="settings-provider-heading"><span className="settings-provider-avatar alternate">智</span><div><strong>智谱 Coding Plan</strong><small>Anthropic API · 密钥已保存</small></div><span className="settings-provider-status">● 已启用</span></div><div className="settings-model-row"><span><strong>GLM-5.3</strong><small>推理档位：low / high / max</small></span><span className="settings-model-chip">内置模型</span></div></div>
  <div className="settings-key-note"><KeyRound size={15}/><span>API 密钥由系统凭据管理器保管，模型列表不会读取或显示密钥。</span></div>
 </div>;
}


function ArchivePreview({archived,projectName,onRestore}:{archived:SessionInfo[];projectName:(id:string)=>string;onRestore:(id:string)=>void}){
 // 按项目分组：组内按最后更新倒序，组间按组内最新会话倒序。
 const byWorkspace=new Map<string,SessionInfo[]>();
 for(const item of [...archived].sort((a,b)=>Date.parse(b.updatedAt)-Date.parse(a.updatedAt))){
  const list=byWorkspace.get(item.workspaceId);
  if(list)list.push(item);else byWorkspace.set(item.workspaceId,[item]);
 }
 if(!archived.length)return <p className="muted settings-archive-empty">还没有归档的会话。在侧栏对会话执行「存档」后，会话会按项目显示在这里。</p>;
 return <>
  {[...byWorkspace.entries()].map(([workspaceId,list])=><section key={workspaceId} aria-label={`已归档会话：${projectName(workspaceId)||workspaceId}`}>
   <div className="archive-group-title"><Folder size={13} aria-hidden="true"/><span>{projectName(workspaceId)||workspaceId}</span><small>{list.length}</small></div>
   <div className="settings-example-list">{list.map(item=><article key={item.id} className="settings-example-item">
    <span className="settings-example-icon"><Archive size={16}/></span>
    <span className="settings-example-copy"><strong>{item.title}</strong><small>归档于 {relativeLabel(item.archivedAt??item.updatedAt,new Date())}</small></span>
    <button type="button" className="settings-manage-button" onClick={()=>onRestore(item.id)}><Undo2 size={13}/>恢复</button>
   </article>)}</div>
  </section>)}
 </>;
}

function PreviewItem({icon,title,description,badge,selected=false}:{icon:ReactNode;title:string;description:string;badge:string;selected?:boolean}){
 return <article className={`settings-example-item${selected?' selected':''}`}><span className="settings-example-icon">{icon}</span><span className="settings-example-copy"><strong>{title}</strong><small>{description}</small></span><span className="settings-example-badge">{badge}</span></article>;
}
