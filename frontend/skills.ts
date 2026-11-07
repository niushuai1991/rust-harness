// 技能补全辅助：光标处 $token 提取与候选过滤（CLI line_editor $ 补全的镜像语义）。
import type {SkillInfo} from './types';

export interface SkillRow {name:string;description:string}
export interface SkillToken {token:string;start:number;end:number}

/// 仅启用且未 broken 的技能进入 `$` 补全候选。
export function skillRowsFrom(infos:SkillInfo[]):SkillRow[] {
 return infos.filter(s=>s.enabled&&!s.brokenReason).map(s=>({name:s.name,description:s.description??''}));
}

/// 光标所在 token 以 $ 开头时返回其区间；光标越过 token 末尾或非 $ 上下文返回 null。
export function skillTokenUnderCursor(text:string,cursor:number):SkillToken|null {
 let start=text.slice(0,cursor).replace(/\S+$/,'').length;
 let end=text.length;
 const whitespace=/\s/.exec(text.slice(start));
 if(whitespace)end=start+whitespace.index;
 if(cursor>end)return null;
 const token=text.slice(start,end);
 return token.startsWith('$')?{token,start,end}:null;
}

/// 大小写不敏感子串过滤：匹配名称与描述；空查询返回全量，保持目录排序。
export function filterSkillRows(rows:SkillRow[],query:string):SkillRow[] {
 const q=query.trim().toLowerCase();
 return rows.filter(row=>!q||row.name.toLowerCase().includes(q)||row.description.toLowerCase().includes(q));
}
