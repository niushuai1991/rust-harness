// 斜杠命令注册表：桌面端可执行子集，与 CLI docs/specs/slash-commands-spec.md 保持镜像。
export interface SlashSpec {name:string;description:string}

export const SLASH_COMMANDS:SlashSpec[] = [
 {name:'clear',description:'重开新会话'},
 {name:'compact',description:'压缩历史为摘要'},
 {name:'init',description:'在项目根生成 AGENTS.md'},
 {name:'plan',description:'制定实施计划'},
 {name:'goal',description:'设定会话目标'},
 {name:'model',description:'查看或切换模型'},
 {name:'diff',description:'查看副本改动'},
 {name:'status',description:'显示会话与运行状态'},
 {name:'help',description:'显示可用命令'},
];

// /init 注入的预置提示词；文本以 crates/harness-cli/src/chat.rs 的 INIT_PROMPT 为单一事实源。
export const INIT_PROMPT = '请分析当前项目并在执行根目录生成 AGENTS.md：\n1. 项目概述（用途、技术栈、构建/测试/运行命令）\n2. 目录结构与关键模块说明\n3. 协作约定（代码风格、提交与验证要求）\n要求内容精炼并全部使用中文；若已存在 AGENTS.md 则读取后在原基础上修订补全。完成后简述你做了什么。';

// /plan、/goal 注入的预置提示词；文本以 crates/harness-cli/src/chat.rs 的 PLAN_PROMPT / GOAL_PROMPT 为单一事实源。
export const PLAN_PROMPT = '请为下面的任务制定实施计划：先阅读相关代码弄清现状，再给出分步骤、可验证的计划，并用 plan_write 工具把计划记录下来；计划未经我确认前不要修改任何文件。';
export const GOAL_PROMPT = '请把下面的内容设定为本会话的目标：用 goal_write 工具记录目标，并简要说明你打算如何推进；未经我确认前不要修改任何文件。';

// 把首命令为 /plan、/goal 的输入展开为预置提示词：带参数拼接任务/目标，无参数提示用法；其余输入原样放行（null）。
export type SlashPromptExpansion={kind:'prompt';text:string}|{kind:'usage';name:'plan'|'goal'}|null;
export function expandSlashPrompt(input:string):SlashPromptExpansion {
 const match=/^\/(plan|goal)(?:\s+([\s\S]+))?$/.exec(input.trim());
 if(!match)return null;
 const name=match[1] as 'plan'|'goal',body=match[2]?.trim();
 if(!body)return {kind:'usage',name};
 return {kind:'prompt',text:`${name==='plan'?PLAN_PROMPT:GOAL_PROMPT}\n\n${name==='plan'?'任务':'目标'}：${body}`};
}

// 过滤候选：exact 命中优先于 prefix 命中，各自保持注册顺序；容忍前导 '/'；空查询返回全量。
export function filterSlashCommands(query:string):SlashSpec[] {
 const q=query.replace(/^\//,'').toLowerCase();
 const exact:SlashSpec[]=[],prefix:SlashSpec[]=[];
 for(const spec of SLASH_COMMANDS){
  if(spec.name===q)exact.push(spec);
  else if(spec.name.startsWith(q))prefix.push(spec);
 }
 return [...exact,...prefix];
}

// 光标仍在首命令 token 内时返回该 token（以 '/' 开头），否则 null。
export function commandTokenUnderCursor(text:string,cursor:number):string|null {
 if(!text.startsWith('/'))return null;
 const match=/\s/.exec(text);
 const end=match?match.index:text.length;
 if(cursor>end)return null;
 return text.slice(0,end);
}
