import {create} from 'zustand';
export interface FormSchema {type:'string'|'number'|'boolean';title?:string;description?:string;default?:string|number|boolean;enum?:(string|number|boolean)[];minimum?:number;maximum?:number;minLength?:number;maxLength?:number}
export type FormValues=Record<string,string|number|boolean>;
export interface PluginContribution{id:string;name:string;settings:Record<string,FormSchema>;values:FormValues;commands:{name:string;title:string;parameters?:{properties?:Record<string,FormSchema>;required?:string[]}}[];themes:Record<string,Record<string,string>>}
export function safeTheme(vars:Record<string,string>){return Object.entries(vars).filter(([k,v])=>k==='--font-family'?['system-ui','sans-serif','serif','monospace'].includes(v):['--bg','--panel','--text','--muted','--border','--accent'].includes(k)&&/^#(?:[a-fA-F0-9]{3}|[a-fA-F0-9]{4}|[a-fA-F0-9]{6}|[a-fA-F0-9]{8})$/.test(v))}

export const usePluginThemes=create<{selections:Record<string,{id:string;name:string}>;select:(workspace:string,id:string,name:string)=>void;clear:(workspace:string)=>void}>(set=>({selections:{},select:(workspace,id,name)=>set(state=>({selections:{...state.selections,[workspace]:{id,name}}})),clear:(workspace)=>set(state=>{const selections={...state.selections};delete selections[workspace];return {selections}})}));
