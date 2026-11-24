// Vitest 通过 Vite 注入样式时保留已解析的文件路径，因此也能识别间接导入。
export function assertNoAppGlobalStyle():void{
 const styles=document.querySelectorAll('style[data-vite-dev-id]');
 for(const style of styles){
  const id=style.getAttribute('data-vite-dev-id')?.replaceAll('\\','/');
  if(id&&/\/frontend\/style\.css(?:\?.*)?$/.test(id)){
   throw new Error('App 交互测试不能加载 frontend/style.css：完整样式会显著拖慢 jsdom。请将需要真实样式的断言放入 App.style.test.tsx。');
  }
 }
}
