import {useEffect,useState} from 'react';
import {Clock,GitCommit,Tag} from 'lucide-react';
import {getVersion} from '@tauri-apps/api/app';

// 开发服务器与测试环境没有真实发布构建：构建时间显示为「开发构建」。
const DEV=import.meta.env.DEV;

export function formatBuildTime(iso:string){
 if(!iso)return '未知';
 const date=new Date(iso);
 if(Number.isNaN(date.getTime()))return iso;
 const pad=(value:number)=>String(value).padStart(2,'0');
 return `${date.getFullYear()}-${pad(date.getMonth()+1)}-${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
}

export function AboutPanel(){
 const [version,setVersion]=useState('');
 useEffect(()=>{let alive=true;getVersion().then(value=>{if(alive)setVersion(value)}).catch(()=>{});return()=>{alive=false}},[]);
 const buildTime=DEV?'开发构建':formatBuildTime(__BUILD_TIME__);
 const commit=__BUILD_COMMIT__||'未知';
 return <div className="settings-example-list settings-about-list">
  <article className="settings-example-item">
   <span className="settings-example-icon"><Tag size={16}/></span>
   <span className="settings-example-copy"><strong>{version||'—'}</strong><small>桌面端版本号</small></span>
   {version&&<span className="settings-example-badge">当前版本</span>}
  </article>
  <article className="settings-example-item">
   <span className="settings-example-icon"><Clock size={16}/></span>
   <span className="settings-example-copy"><strong>{buildTime}</strong><small>构建时间（本地时区）</small></span>
  </article>
  <article className="settings-example-item">
   <span className="settings-example-icon"><GitCommit size={16}/></span>
   <span className="settings-example-copy"><strong>{commit}</strong><small>构建对应的提交 ID</small></span>
  </article>
 </div>;
}
