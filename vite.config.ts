import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
import { execSync } from 'node:child_process';
// 构建信息在打包时注入前端：生产构建写入真实时间与提交号；开发与测试环境由界面显示「开发构建」。
function gitShortHead(){try{return execSync('git rev-parse --short HEAD',{stdio:['ignore','pipe','ignore']}).toString().trim()}catch{return ''}}
export default defineConfig({plugins:[react()],clearScreen:false,server:{port:1420,strictPort:true},define:{__BUILD_TIME__:JSON.stringify(new Date().toISOString()),__BUILD_COMMIT__:JSON.stringify(gitShortHead())},test:{environment:'jsdom',setupFiles:['./frontend/test-setup.ts'],include:['frontend/**/*.{test,spec}.?(c|m)[jt]s?(x)'],css:true}});
