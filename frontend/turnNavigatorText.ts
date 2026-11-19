/** Fit full Unicode grapheme clusters within the measured width. */
export function fitPreview(text:string,width:number,measure:(value:string)=>number):string {
 let result='';
 for(const {segment} of new Intl.Segmenter(undefined,{granularity:'grapheme'}).segment(text)){
  if(measure(result+segment)>width)break;
  result+=segment;
 }
 return result;
}

/** Keep history fetches serialized per session without blocking a different session. */
export function createSessionLoadLock(){
 const pending=new Map<string,Promise<boolean>>();
 return {run(id:string,load:()=>Promise<boolean>):Promise<boolean>{
  const existing=pending.get(id);if(existing)return existing;
  const task=load().finally(()=>{if(pending.get(id)===task)pending.delete(id)});
  pending.set(id,task);return task;
 }};
}
export async function waitForMessage(find:()=>HTMLElement|null,isCurrent:()=>boolean,attempts=30,delay=16):Promise<HTMLElement|null>{
 for(let i=0;i<attempts&&isCurrent();i++){
  const target=find();if(target)return target;
  await new Promise<void>(resolve=>setTimeout(resolve,delay));
 }
 return null;
}
