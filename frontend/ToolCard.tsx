import type {ReactNode} from 'react';
import {Bot,CalendarClock,Camera,ChevronRight,Code,FileCode,FilePlus,FileText,Folder,Globe,Keyboard,ListTodo,MousePointerClick,Pencil,Search,Send,SquareTerminal,Stethoscope,Target,Timer,Wrench} from 'lucide-react';
import type {LucideIcon} from 'lucide-react';
import {DiffView,diffStats,parseUnifiedDiff} from './DiffView';
import {PluginResultCards} from './PluginResultCards';
import {useSidePanel} from './sidePanelStore';
import type {SessionEvent} from './types';

const printable=(value:unknown)=>typeof value==='string'?value:JSON.stringify(value,null,2);
const basename=(path:string)=>path.split(/[\\/]/).filter(Boolean).at(-1)??path;
const dirOf=(path:string)=>{const parts=path.split(/[\\/]/).filter(Boolean);return parts.length>1?parts.slice(0,-1).join('/'):''};

// 通用工具调用节点：与 shellEvents/projectToolEvents 的配对结果对应，两端缺一即为孤儿兜底形态。
export type ToolNode={key:string;call?:SessionEvent;result?:SessionEvent};

const TOOL_META:Record<string,{label:string;icon:LucideIcon}>={
 read:{label:'读取',icon:FileText},
 list_files:{label:'列出文件',icon:Folder},
 search:{label:'搜索',icon:Search},
 web_fetch:{label:'抓取网页',icon:Globe},
 write:{label:'写入',icon:FilePlus},
 edit:{label:'编辑',icon:Pencil},
 todo_write:{label:'待办清单',icon:ListTodo},
 plan_write:{label:'记录计划',icon:ListTodo},
 goal_write:{label:'记录目标',icon:Target},
 agent_spawn:{label:'创建子代理',icon:Bot},
 agent_send:{label:'投递任务',icon:Send},
 agent_fork:{label:'派生会话',icon:Bot},
 agent_status:{label:'子代理状态',icon:Bot},
 agent_wait:{label:'等待子代理',icon:Bot},
 agent_interrupt:{label:'中断子代理',icon:Bot},
 terminal_open:{label:'打开终端',icon:SquareTerminal},
 terminal_write:{label:'写入终端',icon:SquareTerminal},
 terminal_read:{label:'读取终端',icon:SquareTerminal},
 terminal_close:{label:'关闭终端',icon:SquareTerminal},
 schedule_write:{label:'定时任务',icon:CalendarClock},
 schedule_delete:{label:'删除定时任务',icon:CalendarClock},
 schedule_list:{label:'定时任务列表',icon:CalendarClock},
 background_run:{label:'后台任务',icon:Timer},
 background_status:{label:'后台任务状态',icon:Timer},
 background_cancel:{label:'取消后台任务',icon:Timer},
 browser_navigate:{label:'打开网页',icon:Globe},
 browser_click:{label:'点击元素',icon:MousePointerClick},
 browser_type:{label:'输入文本',icon:Keyboard},
 browser_press:{label:'发送按键',icon:Keyboard},
 browser_snapshot:{label:'页面快照',icon:Camera},
 browser_screenshot:{label:'截取屏幕',icon:Camera},
 browser_evaluate:{label:'执行脚本',icon:Code},
 browser_close:{label:'关闭浏览器',icon:Globe},
 lsp_diagnostics:{label:'诊断查询',icon:Stethoscope},
 lsp_hover:{label:'悬停信息',icon:Stethoscope},
 lsp_definition:{label:'定义位置',icon:Stethoscope},
 lsp_references:{label:'引用查找',icon:Stethoscope},
 lsp_symbols:{label:'符号搜索',icon:Stethoscope}
};

function fileEditPath(call?:SessionEvent,result?:SessionEvent):string|null {
 const args=typeof call?.data.arguments==='object'&&call?.data.arguments!==null?call.data.arguments as Record<string,unknown>:typeof result?.data.arguments==='object'&&result?.data.arguments!==null?result.data.arguments as Record<string,unknown>:null;
 if(typeof args?.path==='string'&&args.path)return args.path;
 if(typeof result?.data.path==='string'&&result.data.path)return result.data.path;
 return null;
}

// 标题行语义摘要：优先文件/命令/查询等关键参数，多数场景无需展开即可识别调用意图。
function summaryText(name:string,call?:SessionEvent,result?:SessionEvent):string {
 const args=typeof call?.data.arguments==='object'&&call?.data.arguments!==null?call.data.arguments as Record<string,unknown>:typeof result?.data.arguments==='object'&&result?.data.arguments!==null?result.data.arguments as Record<string,unknown>:null;
 if(!args){
  const fallbackPath=typeof result?.data.path==='string'?result.data.path:null;
  return fallbackPath??'（调用事件缺失）';
 }
 const path=typeof args.path==='string'?args.path:null;
 if(path){
  if(typeof args.line==='number'&&typeof args.character==='number')return `${path}:${args.line}:${args.character}`;
  if(typeof args.offset==='number'&&typeof args.length==='number')return `${path} · ${args.offset}–${args.offset+args.length} 行`;
  return path;
 }
 for(const key of ['command','query','url','task','question','expression','pattern'])if(typeof args[key]==='string'&&args[key])return String(args[key]);
 return printable(args);
}

function durationText(call?:SessionEvent,result?:SessionEvent):string {
 if(!call||!result)return '';
 const ms=new Date(result.createdAt).getTime()-new Date(call.createdAt).getTime();
 if(!Number.isFinite(ms)||ms<0)return '';
 return `${(ms/1000).toFixed(1)}s`;
}

// 通用工具调用单卡：调用与结果合并展示，状态徽章随执行阶段翻转（对齐 turn-reply-presentation 规范）。
export function ToolCard({node,workspaceId,artifact}:{node:ToolNode;workspaceId?:string;artifact?:ReactNode}) {
 const {call,result}=node;
 const name=String(call?.data.name??result?.data.name??'');
 const meta=TOOL_META[name]??{label:name,icon:Wrench};
 const Icon=meta.icon;
 const running=!result;
 const status=String(result?.data.status??'');
 const failed=!running&&(result?.data.isError===true||status==='error'||status==='denied'||status==='unknown'||status==='cancelled');
 const stateClass=running?'running':failed?'failed':'completed';
 const label=running?'运行中':status==='denied'?'已拒绝':status==='unknown'?'结果未知':status==='cancelled'?'已取消':failed?'执行失败':'已完成';
 const time=durationText(call,result);
 const isFileEdit=name==='write'||name==='edit';
 const path=isFileEdit?fileEditPath(call,result):null;
 const diff=isFileEdit&&typeof result?.data.diff==='string'&&result.data.diff?result.data.diff:null;
 const stats=diff?diffStats(parseUnifiedDiff(diff)):null;
 const argsText=call?printable(call.data.arguments):summaryText(name,call,result);
 const content=result?result.data.content:null;
 const resultBody=!result?<pre className="muted">等待结果</pre>
  :name.startsWith('plugin__')?<PluginResultCards workspaceId={workspaceId} content={printable(content)}/>
  :diff?<div className="tool-diff"><DiffView diff={diff}/></div>
  :<pre className={failed?'is-err':undefined}>{printable(content)}</pre>;
 return <details className={`tcard${running?' is-running':''}`}>
  <summary>
   <span className="t-icon" aria-hidden="true"><Icon size={14}/></span>
   <span className="t-title" title={name}>{meta.label}</span>
   <span className="t-detail">
    {path&&<button type="button" className="file-chip" title={path} onClick={event=>{event.preventDefault();useSidePanel.getState().openPreview(path)}}><FileCode size={12}/>{basename(path)}</button>}
    {path&&!!dirOf(path)&&<span className="ft-dir">{dirOf(path)}</span>}
    {(!path||!isFileEdit)&&summaryText(name,call,result)}
   </span>
   {!!stats&&<span className="t-stats"><b className="add">+{stats.add}</b><b className="del">-{stats.del}</b></span>}
   <span className={`pill ${stateClass}`}><span className="dot"/>{label}</span>
   {time&&<span className="t-time">{time}</span>}
   <ChevronRight size={13} className="chev" aria-hidden="true"/>
  </summary>
  <div className="tcard-body">
   <div className="t-sec"><h5>参数</h5><pre>{argsText}</pre></div>
   <div className="t-sec"><h5>{failed&&!running?'错误':'结果'}</h5>{resultBody}</div>
   {artifact}
  </div>
 </details>;
}
