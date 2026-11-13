// 模型能力仅供参考，不代表当前宿主支持原生图片输入。
export function visionLabel(value:boolean|null|undefined):string {
 return value===true?'支持图片输入':value===false?'不支持图片输入':'图片输入能力未知';
}
export function VisionBadge({value}:{value:boolean|null|undefined}) {
 return <span className="chip" title="模型能力参考；当前应用不支持原生图片输入">{visionLabel(value)}</span>;
}
