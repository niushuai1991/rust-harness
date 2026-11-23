import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
import { execSync } from 'node:child_process';
import {readFileSync,readdirSync} from 'node:fs';
import {resolve} from 'node:path';
// PDF.js 的字符映射与标准字体既要用于开发服务器，也要随构建产物离线发布。
function pdfAssets(){
 const root=resolve('node_modules/pdfjs-dist');
 const folders=['cmaps','standard_fonts'];
 return {name:'pdf-assets',
  configureServer(server:import('vite').ViteDevServer){server.middlewares.use((req,res,next)=>{
   const match=req.url?.match(/^\/pdfjs\/(cmaps|standard_fonts)\/([\w.-]+)$/);
   if(!match)return next();
   try{res.end(readFileSync(resolve(root,match[1],match[2])))}catch{res.statusCode=404;res.end()}
  })},
  generateBundle(this:import('rollup').PluginContext){for(const folder of folders)for(const name of readdirSync(resolve(root,folder)))this.emitFile({type:'asset',fileName:`pdfjs/${folder}/${name}`,source:readFileSync(resolve(root,folder,name))})}
 };
}
// 构建信息在打包时注入前端：生产构建写入真实时间与提交号；开发与测试环境由界面显示「开发构建」。
function gitShortHead(){try{return execSync('git rev-parse --short HEAD',{stdio:['ignore','pipe','ignore']}).toString().trim()}catch{return ''}}
export default defineConfig({plugins:[react(),pdfAssets()],clearScreen:false,server:{port:1420,strictPort:true},define:{__BUILD_TIME__:JSON.stringify(new Date().toISOString()),__BUILD_COMMIT__:JSON.stringify(gitShortHead())},test:{environment:'jsdom',setupFiles:['./frontend/test-setup.ts'],include:['frontend/**/*.{test,spec}.?(c|m)[jt]s?(x)'],css:true,
 // 桌面端用例要整棵 App 渲染 + 2 秒轮询，并行跑全量时单测实际耗时会被 CPU 争抢放大到数秒；
 // 默认 5 秒会把「慢」判成「卡死」。放宽到 20 秒，断言时延仍由 waitFor 的 5 秒兜住。
 testTimeout:20000}});
