// 自动化任务的展示辅助纯函数；nextRunAt 为前端镜像计算，仅作预览提示，以后端落库为准。
import type {AutomationInfo,AutomationKind} from './types';
const WD_SHORT=['日','一','二','三','四','五','六'] as const;
const pad=(n:number)=>String(n).padStart(2,'0');
const fmtDate=(d:Date)=>`${d.getFullYear()}-${pad(d.getMonth()+1)}-${pad(d.getDate())}`;
const fmtTime=(d:Date)=>`${pad(d.getHours())}:${pad(d.getMinutes())}`;
export const fmtDateTime=(d:Date)=>`${fmtDate(d)} ${fmtTime(d)}`;
export const fmtDateTimeSec=(d:Date)=>`${fmtDateTime(d)}:${pad(d.getSeconds())}`;
export function parseHhmm(s:string|null|undefined):[number,number]|null{
  if(!s)return null;
  const [h,m]=s.split(':');
  const hh=Number(h),mm=Number(m);
  if(!Number.isInteger(hh)||!Number.isInteger(mm)||hh<0||hh>23||mm<0||mm>59)return null;
  return [hh,mm];
}
/// 单次任务的 datetime-local 默认值：今天 + 当前时:分，秒归零。
export function defaultOnceValue(now=new Date()):string{
  return `${fmtDate(now)}T${fmtTime(now)}:00`;
}
/// Date → datetime-local 值（本地时间，含秒），用于编辑回填。
export function fmtLocalInput(d:Date):string{
  return `${fmtDate(d)}T${fmtTime(d)}:${pad(d.getSeconds())}`;
}
export type ScheduleLike=Pick<AutomationInfo,'kind'|'at'|'time'|'weekdays'|'monthDay'|'month'>;
/// 周期中文描述，与列表卡片一致。
export function fmtSchedule(a:ScheduleLike):string{
  const t=a.time?` ${a.time}`:'';
  switch(a.kind){
    case 'once':return a.at&&Number.isFinite(Date.parse(a.at))?`单次 ${fmtDateTimeSec(new Date(a.at))}`:'单次（未定时间）';
    case 'daily':return `每天${t}`;
    case 'weekly':return `每周${(a.weekdays??[]).slice().sort((x,y)=>x-y).map(d=>WD_SHORT[d%7]).join('、')}${t}`;
    case 'monthly':return `每月 ${a.monthDay??1} 日${t}`;
    case 'yearly':return `每年 ${a.month??1} 月 ${a.monthDay??1} 日${t}`;
  }
}
function localAt(date:Date,h:number,m:number):Date{
  const t=new Date(date);t.setHours(h,m,0,0);return t;
}
/// 计算 from 之后（不含 from）的下一次触发时刻；缺日（短月/平年 2/29）跳过。
export function nextRunAt(a:ScheduleLike,from:Date=new Date()):Date|null{
  const hm=parseHhmm(a.time);
  switch(a.kind){
    case 'once':{
      if(!a.at)return null;
      const t=new Date(a.at);
      return Number.isFinite(t.getTime())?t:null;
    }
    case 'daily':{
      if(!hm)return null;
      for(let days=0;days<=1;days++){
        const d=new Date(from);d.setDate(d.getDate()+days);
        const t=localAt(d,hm[0],hm[1]);
        if(t>from)return t;
      }
      return null;
    }
    case 'weekly':{
      if(!hm)return null;
      const days=a.weekdays??[];
      for(let i=0;i<8;i++){
        const d=new Date(from);d.setDate(d.getDate()+i);
        const wd=d.getDay()===0?7:d.getDay();
        if(!days.includes(wd))continue;
        const t=localAt(d,hm[0],hm[1]);
        if(t>from)return t;
      }
      return null;
    }
    case 'monthly':{
      if(!hm||!a.monthDay)return null;
      for(let i=0;i<48;i++){
        const d=new Date(from.getFullYear(),from.getMonth()+i,1);
        const probe=new Date(d.getFullYear(),d.getMonth(),a.monthDay,hm[0],hm[1],0,0);
        if(probe.getMonth()!==d.getMonth())continue;
        if(probe>from)return probe;
      }
      return null;
    }
    case 'yearly':{
      if(!hm||!a.month||!a.monthDay)return null;
      for(let i=0;i<120;i++){
        const d=new Date(from.getFullYear(),from.getMonth()+i,1);
        if(d.getMonth()+1!==a.month)continue;
        const probe=new Date(d.getFullYear(),d.getMonth(),a.monthDay,hm[0],hm[1],0,0);
        if(probe.getMonth()!==d.getMonth())continue;
        if(probe>from)return probe;
      }
      return null;
    }
  }
}
/// 周期选择项，顺序与面板下拉一致。
export const AUTOMATION_KINDS:{value:AutomationKind;label:string}[]=[
  {value:'once',label:'单次'},{value:'daily',label:'每天'},{value:'weekly',label:'每周'},
  {value:'monthly',label:'每月'},{value:'yearly',label:'每年'},
];
