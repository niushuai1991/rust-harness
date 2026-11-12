import {afterEach,beforeEach,expect,it,vi} from 'vitest';
import {cleanup,fireEvent,render,screen,waitFor} from '@testing-library/react';
import App from './App';
const mocks=vi.hoisted(()=>({command:vi.fn()}));
vi.mock('./api',()=>({desktop:()=>true,command:mocks.command,send:vi.fn(),errorText:(e:unknown)=>e instanceof Error?e.message:String(e)}));
vi.mock('@tauri-apps/plugin-dialog',()=>({open:vi.fn(),save:vi.fn()}));
const provider={id:'zhipu',name:'智谱 Coding Plan',builtin:true,compatibility:'bigModel',endpoints:[{apiMode:'anthropic',baseUrl:'https://open.bigmodel.cn/api/anthropic'}],defaultModel:'glm-5.3',credentialRef:'glm-key',proxy:null,enabled:true,
 builtinModels:[{id:'glm-5.3',name:'GLM-5.3',efforts:['low','high','max']}],disabledModels:[]};
const model={id:'glm-5.3',name:'GLM-5.3',compatibility:'bigModel',providerId:'zhipu',apiMode:'anthropic',maxOutputTokens:null,baseUrl:'https://open.bigmodel.cn/api/anthropic',model:'glm-5.3',credentialRef:'glm-key',proxy:null,efforts:['low','high','max'],effort:null,enabled:true,builtin:true};
const builtin={id:'builtin:default',name:'默认（无覆盖）',description:'不覆盖模型、档位与审批策略，行为与未选预设一致',modelId:null,effort:null,approval:null,broken:null};
const good={id:'refactor',name:'批量重构',description:'自动审批跑批量任务',modelId:'glm-5.3',effort:'high',approval:'auto',broken:null};
const stale={id:'stale',name:'失效预设',description:null,modelId:'ghost',effort:null,approval:null,broken:'模型 ghost 不存在'};
let presets:any[]=[];
function reset(){presets=[structuredClone(builtin),structuredClone(good),structuredClone(stale)]}
beforeEach(()=>{reset();mocks.command.mockReset();mocks.command.mockImplementation(async(name:string,args:any)=>{
 if(name==='workspace_list')return [{id:'w',sourcePath:'D:/project',replicaPath:'D:/copy',createdAt:''}];
 if(name==='model_list')return [structuredClone(model)];
 if(name==='provider_list')return [structuredClone(provider)];
 if(name==='default_selection_get')return null;
 if(name==='session_list')return [];
 if(name==='session_events')return [];
 if(name==='approval_list')return [];
 if(name==='preset_list')return structuredClone(presets);
 if(name==='preset_save'){const next=structuredClone(args.preset);presets=[...presets.filter(p=>p.id!==next.id),next];return structuredClone(next)}
 if(name==='preset_remove'){presets=presets.filter(p=>p.id!==args.id);return null}
 return [];
})});
afterEach(cleanup);
async function openPanel(){render(<App/>);fireEvent.click(screen.getByRole('button',{name:'设置'}));fireEvent.click(await screen.findByRole('button',{name:'能力预设'}));fireEvent.click(screen.getByRole('button',{name:'打开现有面板'}));await screen.findByRole('button',{name:/批量重构/})}

it('内置预设只读置顶，用户预设与不可用标记可见',async()=>{
 await openPanel();
 const rows=screen.getAllByRole('button',{name:/默认（无覆盖）/});
 expect(rows.length).toBeGreaterThan(0);
 expect(screen.getByText(/不可用（模型 ghost 不存在）/)).toBeInTheDocument();
 expect(screen.queryByRole('button',{name:'删除预设'})).not.toBeInTheDocument();
});

it('新建预设提交完整载荷并刷新列表',async()=>{
 await openPanel();
 fireEvent.click(screen.getByRole('button',{name:'新建预设'}));
 fireEvent.change(screen.getByLabelText('预设 ID'),{target:{value:'fast'}});
 fireEvent.change(screen.getByLabelText('显示名称'),{target:{value:'快速响应'}});
 fireEvent.change(screen.getByLabelText('模型'),{target:{value:'glm-5.3'}});
 fireEvent.change(screen.getByLabelText('预设档位'),{target:{value:'high'}});
 fireEvent.change(screen.getByLabelText('审批策略'),{target:{value:'auto'}});
 fireEvent.click(screen.getByRole('button',{name:'保存预设'}));
 await screen.findByText(/已保存预设/);
 expect(mocks.command).toHaveBeenCalledWith('preset_save',{preset:{id:'fast',name:'快速响应',description:null,modelId:'glm-5.3',effort:'high',approval:'auto'}});
 await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('preset_list'));
});

it('编辑与删除用户预设走同一表单',async()=>{
 await openPanel();
 fireEvent.click(screen.getByRole('button',{name:/批量重构/}));
 const idInput=screen.getByLabelText('预设 ID') as HTMLInputElement;
 expect(idInput.value).toBe('refactor');
 expect(screen.getByLabelText('说明')).toBeInTheDocument();
 fireEvent.change(screen.getByLabelText('显示名称'),{target:{value:'重构套餐'}});
 fireEvent.click(screen.getByRole('button',{name:'保存预设'}));
 await screen.findByText(/已保存预设/);
 expect(mocks.command).toHaveBeenCalledWith('preset_save',{preset:expect.objectContaining({id:'refactor',name:'重构套餐',approval:'auto'})});
 await waitFor(()=>expect(screen.getByRole('button',{name:'删除预设'})).toBeEnabled());
 fireEvent.click(screen.getByRole('button',{name:'删除预设'}));
 await waitFor(()=>expect(mocks.command).toHaveBeenCalledWith('preset_remove',{id:'refactor'}));
 await waitFor(()=>expect(screen.queryByRole('button',{name:'删除预设'})).not.toBeInTheDocument());
});
