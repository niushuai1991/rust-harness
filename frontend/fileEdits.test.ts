import {describe,expect,it} from 'vitest';
import {groupFileEdits,summarizeFileEdits,type FileEditGroup} from './fileEdits';
import type {SessionEvent} from './types';

let seq=0;
function event(id:string,kind:string,data:Record<string,unknown>):SessionEvent{
 seq+=1;
 return {id,sessionId:'s',seq:String(seq),kind,data,createdAt:''};
}
function call(id:string,path:string|null,name:'edit'|'write'|'read'='edit'):SessionEvent{
 return event(id,'tool.call',{id,name,arguments:path?{path}:{}});
}
function result(id:string,callId:string,path:string|null,diff:string|null=null,isError=false,name:'edit'|'write'='edit'):SessionEvent{
 return event(id,'tool.result',{callId,name,...(path?{path}:{}),diff,isError,status:isError?'error':'completed',content:isError?'替换失败':`已修改 ${path??''}`});
}
const DIFF_ONE=['--- a/src/a.ts','+++ b/src/a.ts','@@ -1,1 +1,2 @@','-old','+new','+extra'].join('\n');
const DIFF_TWO=['--- a/src/a.ts','+++ b/src/a.ts','@@ -1,2 +1,1 @@','-first','-second','+merged'].join('\n');

describe('groupFileEdits',()=>{
 it('把紧邻的同路径多次编辑合并为一个组',()=>{
  const nodes=groupFileEdits([
   call('c1','src/a.ts'),
   result('r1','c1','src/a.ts',DIFF_ONE),
   call('c2','src/a.ts'),
   result('r2','c2','src/a.ts',DIFF_TWO),
  ]);
  expect(nodes).toHaveLength(1);
  expect(nodes[0]).toMatchObject({kind:'group',key:'c1',group:{path:'src/a.ts'}});
  if(nodes[0].kind==='group')expect(nodes[0].group.events.map(item=>item.id)).toEqual(['c1','r1','c2','r2']);
 });

 it('单次编辑保持逐事件展示，避免改变既有两行结构',()=>{
  const nodes=groupFileEdits([call('c1','src/a.ts'),result('r1','c1','src/a.ts',DIFF_ONE)]);
  expect(nodes.map(node=>node.kind)).toEqual(['event','event']);
  expect(nodes.map(node=>node.kind==='event'?node.event.id:'')).toEqual(['c1','r1']);
 });

 it('不同文件不合并',()=>{
  const nodes=groupFileEdits([
   call('c1','src/a.ts'),
   result('r1','c1','src/a.ts',DIFF_ONE),
   call('c2','src/b.ts'),
   result('r2','c2','src/b.ts',DIFF_TWO),
  ]);
  expect(nodes).toHaveLength(4);
  expect(nodes.every(node=>node.kind==='event')).toBe(true);
 });

 it('中间夹其他工具或消息时切分，不重排时间线',()=>{
  const nodes=groupFileEdits([
   call('c1','src/a.ts'),
   result('r1','c1','src/a.ts',DIFF_ONE),
   call('c2','src/b.ts','read'),
   event('x','assistant.message',{content:'中间说明'}),
   call('c3','src/a.ts'),
   result('r3','c3','src/a.ts',DIFF_TWO),
  ]);
  expect(nodes).toHaveLength(6);
  expect(nodes.every(node=>node.kind==='event')).toBe(true);
 });

 it('同一文件中途断开后，只合并各自的紧邻区间',()=>{
  const nodes=groupFileEdits([
   call('c1','src/a.ts'),
   result('r1','c1','src/a.ts',DIFF_ONE),
   call('c2','src/a.ts'),
   result('r2','c2','src/a.ts',DIFF_TWO),
   event('x','todo.state',{items:[]}),
   call('c3','src/a.ts'),
   result('r3','c3','src/a.ts',DIFF_ONE),
   call('c4','src/a.ts'),
   result('r4','c4','src/a.ts',DIFF_TWO),
  ]);
  expect(nodes.map(node=>node.kind)).toEqual(['group','event','group']);
 });

 it('结果事件缺路径时按 callId 继承相邻调用路径',()=>{
  const nodes=groupFileEdits([
   call('c1','src/a.ts'),
   result('r1','c1',null,DIFF_ONE),
   call('c2','src/a.ts'),
   result('r2','c2',null,DIFF_TWO),
  ]);
  expect(nodes).toHaveLength(1);
  expect(nodes[0]).toMatchObject({kind:'group',group:{path:'src/a.ts'}});
 });

 it('结果事件缺名称时按 callId 继承调用名称',()=>{
  const missingName=(id:string,callId:string,diff:string)=>event(id,'tool.result',{callId,path:'src/a.ts',diff,isError:false,status:'completed',content:'已修改'});
  const nodes=groupFileEdits([
   call('c1','src/a.ts'),
   missingName('r1','c1',DIFF_ONE),
   call('c2','src/a.ts'),
   missingName('r2','c2',DIFF_TWO),
  ]);
  expect(nodes).toHaveLength(1);
 });
 it('write 与 edit 指向同一文件时也合并',()=>{
  const nodes=groupFileEdits([
   call('c1','docs/readme.md','write'),
   result('r1','c1','docs/readme.md',DIFF_ONE,false,'write'),
   call('c2','docs/readme.md','edit'),
   result('r2','c2','docs/readme.md',DIFF_TWO),
  ]);
  expect(nodes).toHaveLength(1);
  if(nodes[0].kind==='group')expect(summarizeFileEdits(nodes[0].group).label).toBe('修改');
 });

 it('read 等其他工具不会进入文件编辑组',()=>{
  const nodes=groupFileEdits([
   call('c1','src/a.ts','read'),
   event('r1','tool.result',{callId:'c1',name:'read',path:'src/a.ts',content:'文件内容',isError:false,status:'completed'}),
  ]);
  expect(nodes).toHaveLength(2);
  expect(nodes.every(node=>node.kind==='event')).toBe(true);
 });
});

describe('summarizeFileEdits',()=>{
 const group=(events:SessionEvent[]):FileEditGroup=>({path:'src/a.ts',events});

 it('汇总操作数、状态与各次 diff 的增删行数',()=>{
  const summary=summarizeFileEdits(group([
   call('c1','src/a.ts'),
   result('r1','c1','src/a.ts',DIFF_ONE),
   call('c2','src/a.ts'),
   result('r2','c2','src/a.ts',DIFF_TWO),
  ]));
  expect(summary).toMatchObject({path:'src/a.ts',label:'编辑',operations:2,pending:0,failed:0,add:3,del:3,hasStats:true});
  expect(summary.results.map(item=>item.id)).toEqual(['r1','r2']);
 });

 it('调用尚无结果时计入待执行，状态可显示进行中',()=>{
  const summary=summarizeFileEdits(group([
   call('c1','src/a.ts'),
   result('r1','c1','src/a.ts',DIFF_ONE),
   call('c2','src/a.ts'),
  ]));
  expect(summary).toMatchObject({operations:2,pending:1,failed:0,add:2,del:1,hasStats:true});
 });

 it('部分失败与全部失败分别计数，失败结果无 diff 不参与统计',()=>{
  const partial=summarizeFileEdits(group([
   call('c1','src/a.ts'),
   result('r1','c1','src/a.ts',DIFF_ONE),
   call('c2','src/a.ts'),
   result('r2','c2','src/a.ts',null,true),
  ]));
  expect(partial).toMatchObject({operations:2,pending:0,failed:1,add:2,del:1,hasStats:true});
  const allFailed=summarizeFileEdits(group([
   call('c1','src/a.ts'),
   result('r1','c1','src/a.ts',null,true),
   call('c2','src/a.ts'),
   result('r2','c2','src/a.ts',null,true),
  ]));
  expect(allFailed).toMatchObject({operations:2,pending:0,failed:2,add:0,del:0,hasStats:false});
 });

 it('纯 write 组标记为写入，纯 edit 组标记为编辑',()=>{
  const write=summarizeFileEdits(group([
   call('c1','src/a.ts','write'),
   result('r1','c1','src/a.ts',DIFF_ONE,false,'write'),
   call('c2','src/a.ts','write'),
   result('r2','c2','src/a.ts',DIFF_TWO,false,'write'),
  ]));
  expect(write.label).toBe('写入');
  const edit=summarizeFileEdits(group([
   call('c1','src/a.ts'),
   result('r1','c1','src/a.ts',DIFF_ONE),
   call('c2','src/a.ts'),
   result('r2','c2','src/a.ts',DIFF_TWO),
  ]));
  expect(edit.label).toBe('编辑');
 });
});
