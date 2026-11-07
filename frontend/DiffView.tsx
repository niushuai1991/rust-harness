import {useMemo} from 'react';
export type DiffLineType='hunk'|'add'|'del'|'ctx'|'note';
export interface DiffLine {type:DiffLineType;oldNo:number|null;newNo:number|null;text:string}
const HUNK_RE=/^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/;
// 解析后端 similar 生成的 unified diff 文本（crates/harness-platform/src/diffview.rs）。
// 首个 hunk 之前的 ---/+++ 是文件头；hunk 之后以 -/+ 开头的行一律是正文，避免 SQL "--" 等内容误判。
export function parseUnifiedDiff(text:string):DiffLine[] {
 const lines:DiffLine[]=[];let oldNo=0,newNo=0,oldCounting=false,newCounting=false,sawHunk=false;
 for(const raw of text.split('\n')) {
  if(!sawHunk&&(raw.startsWith('--- ')||raw.startsWith('+++ ')))continue;
  const hunk=HUNK_RE.exec(raw);
  if(hunk){sawHunk=true;lines.push({type:'hunk',oldNo:null,newNo:null,text:raw});oldNo=Number(hunk[1]);newNo=Number(hunk[3]);oldCounting=Number(hunk[2]??'1')>0;newCounting=Number(hunk[4]??'1')>0;continue}
  if(raw.startsWith(' '))lines.push({type:'ctx',oldNo:oldCounting?oldNo++:null,newNo:newCounting?newNo++:null,text:raw.slice(1)});
  else if(raw==='')lines.push({type:'ctx',oldNo:oldCounting?oldNo++:null,newNo:newCounting?newNo++:null,text:''});
  else if(raw.startsWith('+'))lines.push({type:'add',oldNo:null,newNo:newCounting?newNo++:null,text:raw.slice(1)});
  else if(raw.startsWith('-'))lines.push({type:'del',oldNo:oldCounting?oldNo++:null,newNo:null,text:raw.slice(1)});
  else lines.push({type:'note',oldNo:null,newNo:null,text:raw});
 }
 // split 尾部空串：diff 文本以换行结束时不算上下文行
 while(lines.length&&lines.at(-1)!.type==='ctx'&&lines.at(-1)!.text===''&&text.endsWith('\n'))lines.pop();
 return lines;
}
export function diffStats(lines:DiffLine[]):{add:number;del:number} {
 let add=0,del=0;for(const l of lines){if(l.type==='add')add++;else if(l.type==='del')del++}
 return {add,del};
}
export function DiffView({diff}:{diff:string}) {
 const lines=useMemo(()=>parseUnifiedDiff(diff),[diff]);
 return <div className="diff-body" aria-label="行级差异">{lines.map((l,index)=>{
  if(l.type==='hunk')return <div key={index} className="diff-hunk">{l.text}</div>;
  if(l.type==='note')return <div key={index} className="diff-note">{l.text}</div>;
  const sign=l.type==='add'?'+':l.type==='del'?'-':' ';
  return <div key={index} className={`diff-line ${l.type}`}><span className="diff-no">{l.oldNo??''}</span><span className="diff-no">{l.newNo??''}</span><span className="diff-text">{sign+l.text}</span></div>;
 })}</div>;
}
