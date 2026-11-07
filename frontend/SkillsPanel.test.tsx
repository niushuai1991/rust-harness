import {afterEach,beforeEach,expect,it,vi} from 'vitest';
import {cleanup,fireEvent,render,screen,waitFor,within} from '@testing-library/react';
import {EventCard,Modal} from './App';
import {SkillsPanel} from './SkillsPanel';
import type {SkillInfo} from './types';
const mocks=vi.hoisted(()=>({command:vi.fn()}));
vi.mock('./api',()=>({desktop:()=>true,command:mocks.command,send:vi.fn(),errorText:(e:unknown)=>e instanceof Error?e.message:String(e)}));

const review:SkillInfo={name:'review',description:'代码审查流程',path:'.agents/skills/review/SKILL.md',scope:'project',enabled:true,brokenReason:null};
const homeSkill:SkillInfo={name:'deploy',description:'部署应用',path:'C:/Users/u/.agents/skills/deploy/SKILL.md',scope:'user',enabled:false,brokenReason:null};
const broken:SkillInfo={name:'bad name',description:null,path:'.agents/skills/bad name/SKILL.md',scope:'project',enabled:true,brokenReason:'技能名 bad name 非法'};
let skills:SkillInfo[]=[];
beforeEach(()=>{skills=[structuredClone(review),structuredClone(homeSkill),structuredClone(broken)];mocks.command.mockReset();
 mocks.command.mockImplementation(async(name:string,args:any)=>{
  if(name==='skill_list')return structuredClone(skills);
  if(name==='skill_set_enabled'){skills=skills.map(s=>s.path===args.path?{...s,enabled:args.enabled}:s);return null}
  return [];
 })});
afterEach(cleanup);

it('技能面板渲染目录行：scope 徽标、禁用与 broken 标记',async()=>{
 render(<SkillsPanel workspaceId="w" onClose={()=>{}}/>);
 expect(await screen.findByText('$review')).toBeInTheDocument();
 expect(screen.getByText('$deploy')).toBeInTheDocument();
 expect(screen.getByText('不可用（技能名 bad name 非法）')).toBeInTheDocument();
 expect(screen.getByText('已禁用')).toBeInTheDocument();
 expect(screen.getByText('.agents/skills/review/SKILL.md')).toBeInTheDocument();
});

it('空目录显示引导文案',async()=>{
 skills=[];
 render(<SkillsPanel workspaceId="w" onClose={()=>{}}/>);
 expect(await screen.findByText(/没有可用技能/)).toBeInTheDocument();
});

it('启停开关调用 skill_set_enabled 并刷新列表',async()=>{
 render(<SkillsPanel workspaceId="w" onClose={()=>{}}/>);
 await screen.findByText('$review');
 const buttons=screen.getAllByRole('button',{name:'禁用'});
 fireEvent.click(buttons[0]);
 await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('skill_set_enabled',{workspaceId:'w',path:review.path,enabled:false}));
 await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('skill_list',{workspaceId:'w'}));
});

it('broken 技能不可启停，其余行按钮可用',async()=>{
 render(<SkillsPanel workspaceId="w" onClose={()=>{}}/>);
 await screen.findByText('$bad name');
 const brokenRow=screen.getByText('$bad name').closest('article')!;
 expect(within(brokenRow).getByRole('button')).toBeDisabled();
 const reviewRow=screen.getByText('$review').closest('article')!;
 expect(within(reviewRow).getByRole('button',{name:'禁用'})).toBeEnabled();
});

it('EventCard 渲染 skill.loaded 卡片并可展开正文',()=>{
 render(<Modal title="t" onClose={()=>{}}><EventCard event={{id:'1',sessionId:'s',seq:'1',kind:'skill.loaded',data:{name:'review',path:review.path,content:'审查正文内容'},createdAt:''}}/></Modal>);
 const summary=screen.getByText(/已加载技能/);
 expect(summary).toBeInTheDocument();
 fireEvent.click(summary.closest('summary')!);
 expect(screen.getByText(/审查正文内容/)).toBeInTheDocument();
});
