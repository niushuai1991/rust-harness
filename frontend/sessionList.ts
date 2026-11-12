// 会话列表「最近 / 项目」两视图的纯函数：时间分组、排序、过滤与相对时间标签。
import type {SessionInfo,WorkspaceInfo} from './types';

export type ViewMode='recent'|'project';
/** 时间分组：0 今天、1 昨天、2 近 7 天、3 更早。 */
export type TimeGroup=0|1|2|3;
export const GROUP_LABELS=['今天','昨天','近 7 天','更早'] as const;

const DAY_MS=86400000;
const startOfDay=(d:Date)=>new Date(d.getFullYear(),d.getMonth(),d.getDate()).getTime();
const WEEKDAYS=['周日','周一','周二','周三','周四','周五','周六'];

export function timeGroupOf(updatedAt:string,now:Date):TimeGroup{
 const time=Date.parse(updatedAt);
 if(!Number.isFinite(time))return 3;
 const days=Math.floor((startOfDay(now)-startOfDay(new Date(time)))/DAY_MS);
 if(days<=0)return 0;
 if(days===1)return 1;
 return days<7?2:3;
}

export function relativeLabel(updatedAt:string,now:Date):string{
 const time=Date.parse(updatedAt);
 if(!Number.isFinite(time))return '';
 const minutes=Math.floor(Math.max(0,now.getTime()-time)/60000);
 if(minutes<1)return '刚刚';
 const date=new Date(time),group=timeGroupOf(updatedAt,now);
 if(group===0)return minutes<60?`${minutes} 分钟前`:`${Math.floor(minutes/60)} 小时前`;
 if(group===1)return `昨天 ${String(date.getHours()).padStart(2,'0')}:${String(date.getMinutes()).padStart(2,'0')}`;
 if(group===2)return WEEKDAYS[date.getDay()];
 const md=`${date.getMonth()+1}月${date.getDate()}日`;
 return date.getFullYear()===now.getFullYear()?md:`${date.getFullYear()}年${md}`;
}

export function filterByTitle(sessions:SessionInfo[],query:string):SessionInfo[]{
 const q=query.trim().toLowerCase();
 if(!q)return sessions;
 return sessions.filter(s=>s.title.toLowerCase().includes(q));
}

export interface RecentGroup{group:TimeGroup;sessions:SessionInfo[];}
/** 输入需已按 updatedAt 倒序（后端保证）；相邻同组合并，避免打乱组内顺序。 */
export function groupByTime(sessions:SessionInfo[],now:Date):RecentGroup[]{
 const groups:RecentGroup[]=[];
 for(const s of sessions){
  const group=timeGroupOf(s.updatedAt,now);
  const last=groups[groups.length-1];
  if(last&&last.group===group)last.sessions.push(s);
  else groups.push({group,sessions:[s]});
 }
 if(!groups.some(group=>group.group===0))groups.unshift({group:0,sessions:[]});
 return groups;
}

export interface ProjectGroup{workspace:WorkspaceInfo;sessions:SessionInfo[];}
/**
 * 工作区成组；置顶排最前，同状态内有会话的按组内最新会话倒序。
 * includeEmpty=true 时无会话的项目也保留组头（排在同状态有会话组之后，空组间按名称排序），
 * 供任务列表在项目会话被全部存档后仍可回到该项目；缺省不出现（搜索过滤场景）。
 */
export function groupByProject(sessions:SessionInfo[],workspaces:WorkspaceInfo[],includeEmpty=false):ProjectGroup[]{
 const byWorkspace=new Map<string,SessionInfo[]>();
 for(const s of sessions){
  const list=byWorkspace.get(s.workspaceId);
  if(list)list.push(s);else byWorkspace.set(s.workspaceId,[s]);
 }
 const groups:ProjectGroup[]=[];
 for(const workspace of workspaces){
  const list=byWorkspace.get(workspace.id);
  if(list&&list.length)groups.push({workspace,sessions:list});
  else if(includeEmpty)groups.push({workspace,sessions:[]});
 }
 return groups.sort((a,b)=>{
  if(a.workspace.pinned!==b.workspace.pinned)return a.workspace.pinned?-1:1;
  if(!a.sessions.length||!b.sessions.length){
   if(a.sessions.length)return -1;
   if(b.sessions.length)return 1;
   return a.workspace.name.localeCompare(b.workspace.name);
  }
  return Date.parse(b.sessions[0].updatedAt)-Date.parse(a.sessions[0].updatedAt);
 });
}
