/// 浮层水平自适应：返回相对锚点容器左缘的 left（px）。
/// 默认从锚点左缘向左 6px 起向右展开；视口右缘放不下时整体左移，
/// 直到右侧留出 margin；视口窄于浮层时退化为左缘对齐 margin。
/// 测量值缺失（未完成布局，浮层宽或视口宽为 0）时返回 null，由调用方沿用 CSS 默认定位。
export function fitPopoverLeft(anchorLeft:number,popoverWidth:number,viewportWidth:number,preferredLeft=-6,margin=8):number|null{
 if(!(popoverWidth>0)||!(viewportWidth>0))return null;
 const raw=anchorLeft+preferredLeft;
 const maxLeft=viewportWidth-margin-popoverWidth;
 return Math.max(margin,Math.min(raw,maxLeft))-anchorLeft;
}
