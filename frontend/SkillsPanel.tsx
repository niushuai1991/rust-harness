import {useEffect,useRef,useState} from 'react';
import {command,desktop,errorText} from './api';
import {Modal} from './App';
import type {SkillInfo} from './types';
const SCOPE_LABELS:Record<string,string>={project:'项目',user:'用户'};
export function SkillsPanel({workspaceId,onClose}:{workspaceId:string;onClose:()=>void}) {
 const [skills,setSkills]=useState<SkillInfo[]>([]),[error,setError]=useState(''),[busy,setBusy]=useState(false);
 const alive=useRef(true);
 async function load(){const list=await command<SkillInfo[]>('skill_list',{workspaceId});if(alive.current)setSkills(list)}
 useEffect(()=>{alive.current=true;setError('');void load().catch(e=>{if(alive.current)setError(errorText(e))});return()=>{alive.current=false}},[workspaceId]);
 async function toggle(skill:SkillInfo){if(busy||!desktop())return;setBusy(true);setError('');
  try{await command('skill_set_enabled',{workspaceId,path:skill.path,enabled:!skill.enabled});await load()}
  catch(e){if(alive.current)setError(errorText(e))}finally{if(alive.current)setBusy(false)}}
 return <Modal wide title="技能" onClose={onClose}>
  <p className="muted">技能是 <code>.agents/skills/名称/SKILL.md</code>（项目级）或 <code>~/.agents/skills/</code>（用户级）中的指令文件。系统提示会列出技能目录供模型按需读取；在消息中输入 <code>$名称</code> 可显式注入技能全文。禁用后技能不进目录、不再响应 <code>$</code> 提及。</p>
  {error&&<p role="alert" className="inline-error">{error}</p>}
  <div className="plugin-list">
   {!skills.length&&<p className="empty-small muted">当前工作副本与用户目录没有可用技能。</p>}
   {skills.map(skill=><article key={skill.path}>
    <div className="plugin-heading"><strong>${skill.name}</strong><small>{SCOPE_LABELS[skill.scope]}</small>{skill.brokenReason&&<em className="off-badge">不可用（{skill.brokenReason}）</em>}{!skill.enabled&&!skill.brokenReason&&<em className="off-badge">已禁用</em>}</div>
    <p>{skill.description??'（无描述）'}</p>
    <small className="muted">{skill.path}</small>
    <div className="plugin-actions"><small>{skill.brokenReason?'修复技能文件后可用':skill.enabled?'目录可见 · 可被 $ 提及':'已从目录隐藏'}</small><button disabled={busy||!!skill.brokenReason||!desktop()} onClick={()=>toggle(skill)}>{skill.enabled?'禁用':'启用'}</button></div>
   </article>)}
  </div>
 </Modal>;
}
